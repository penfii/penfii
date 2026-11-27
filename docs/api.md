# Using the API

This page tells an integration developer where the API is described and what a first request needs. It is an entry point, not a reference: the reference is the OpenAPI description, and nothing here repeats it.

Penfii is at v0.5.0, an early release. **The HTTP API can change between 0.x releases** and there is no compatibility guarantee; read the [changelog](../CHANGELOG.md) before you upgrade a client. See [Known limitations](known-limitations.md#stability).

## Where the API is described

[`server/api/openapi.json`](../server/api/openapi.json) lists every route with its parameters, request and response bodies and error answers. It is maintained by hand, and a server test fails when a route and the file disagree, so it is the source to trust (see [Changing the API](development.md#changing-the-api)). Penfii does not serve an interactive viewer; open the file in any OpenAPI viewer or import it into a client generator. Its `servers` entry is `http://localhost:8080`, the address of the [Compose install](self-hosting.md); use your own `PENFII_PUBLIC_URL` origin instead.

## Signing in

The API has one authentication scheme: a session cookie, `penfii_session`, that a person's sign-in sets. There is no API key. `POST /v1/session/login` with `{"email": "...", "password": "..."}` answers `200` with the person and sets the cookie; send the cookie back on later requests. For an account with two-step verification on, a right password without a code answers `401` with `"code": "mfa_required"`; send the same request again with `"code": "<authenticator or recovery code>"`. Add `"keepSignedIn": true` to ask for a long session (it ends 30 days after sign-in at the latest and after 7 days without activity, unless the operator changed that; the cookie's `Max-Age` says how long, and `GET /v1/auth/providers` has `longSession` with `enabled` and the two limits); without it, or when the operator has the feature off, the session is the ordinary one. `POST /v1/auth/{provider}/start` takes `keepSignedIn` in the same way for `"intent": "login"`, and `GET /v1/me/sessions` marks a long session with `longLived` and its `absoluteExpiresAt`. See [Long sessions](configuration.md#long-sessions).

Set `PENFII_URL` to your Penfii origin first (for example `PENFII_URL=http://localhost:8080`).

```sh
curl -c cookies.txt -X POST "$PENFII_URL/v1/session/login" \
  -H 'X-Penfii-Client: 1' -H 'Content-Type: application/json' \
  -d '{"email": "you@example.com", "password": "..."}'

curl -b cookies.txt "$PENFII_URL/v1/workspaces"
```

A request that changes something has to carry two more things: the header `X-Penfii-Client: 1` on every `POST`, `PUT`, `PATCH` and `DELETE` (a request without it is refused with `403` and the code `csrf`; the OpenAPI file declares it on only some operations, but the server requires it on all of them except incoming webhooks, `/mcp` and the OAuth token, revoke and registration endpoints), and, where the operation lists it as a required parameter, an `Idempotency-Key` header with a fresh value for each new change. Reads need neither. A missing key is refused with `400 validation`. Use a new key for each new change, and send the same key again only to retry that same request.

A caller acts as that person and sees only what they may see. A thing the person may not see answers `404`, not `403`.

## Errors

A failed request answers with a JSON body of at least `{"code": "...", "message": "..."}`. Branch on `code`, which is stable enough to test for, and show `message` only to developers. Some answers add fields (`version_conflict` carries the current `body`, for example); the operation's responses in the OpenAPI file say which.

| Status | Code | Meaning |
|---|---|---|
| `401` | `unauthenticated` | No session, an expired session, or a wrong email or password at login |
| `403` | `csrf` | A change was sent without `X-Penfii-Client: 1` |
| `503` | `db_unavailable` | The database was briefly unavailable; the server does not retry for you, so try again |

Other statuses and codes (`400 validation`, `404`, `409` conflicts and the like) are listed per operation.

## Other ways in

- **MCP.** AI clients that speak the Model Context Protocol connect to `PENFII_PUBLIC_URL/mcp` with OAuth and PKCE, on behalf of a person who approves them in the browser. MCP is off by default. Turn it on and set its policy as described in [the MCP gateway](self-hosting.md#the-mcp-gateway-optional) and [MCP gateway settings](configuration.md#mcp-gateway). The [README](../README.md#connect-an-ai-client-with-mcp) says what the gateway does.
- **Incoming webhooks.** A channel manager can create a webhook that posts plain text into one channel without a session, using a secret in the path or a signature. See [Incoming webhooks](configuration.md#incoming-webhooks).
- **Changing the API itself.** Contributors who add or change a route follow [Changing the API](development.md#changing-the-api), which also covers the client's copy of the description.
