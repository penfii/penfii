/** The sign-in page checks for an existing session. A 401 from this one endpoint is expected. */
export function isSignInSessionCheck(pageUrl: string, requestUrl: string): boolean {
  return new URL(pageUrl).pathname === '/login' && new URL(requestUrl).pathname === '/v1/me'
}
