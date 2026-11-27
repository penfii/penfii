# User guide

한국어: [사용자 가이드](user-guide.ko.md)

This guide follows the work most people do in Penfii v0.3.1, from joining a workspace to finding things again later. Button and menu names are the ones the English screens show; the [Korean guide](user-guide.ko.md) says the same things with the Korean names. To install or run Penfii, see [Self-hosting](self-hosting.md) instead.

What you can do depends on your roles, which are noted where they matter and summarized in [Who can do what](#who-can-do-what).

## Contents

1. [Join a workspace](#1-join-a-workspace)
2. [Talk in channels and threads](#2-talk-in-channels-and-threads)
3. [Turn a message into a ticket](#3-turn-a-message-into-a-ticket)
4. [Manage tickets: assignee, State, Board and Table](#4-manage-tickets-assignee-state-board-and-table)
5. [Link tickets and documents](#5-link-tickets-and-documents)
6. [Search](#6-search)
7. [Inbox, Watch and Later](#7-inbox-watch-and-later)
8. [Language and personal settings](#8-language-and-personal-settings)
9. [Who can do what](#who-can-do-what)

## 1. Join a workspace

People join a workspace by invitation only; there is no public sign-up.

**If you were invited**

1. Open the invitation link you were given. It works for the e-mail address the invitation names. A link is shown to the inviter only once, so ask for a new one if yours is lost or has expired.
2. On **Accept invitation**, enter a **Display name** and a **Password** (15 to 128 characters), then choose **Join**. If that e-mail address already has an account, the page asks you to **Sign in to accept** instead, and brings you back to the invitation afterwards.
3. Choose **Continue** to reach **Workspaces**, then **Open** your workspace. Without an invitation, a new account sees **Waiting for an invitation**.

On later visits, sign in with **Log in**. When the server sends e-mail, **Forgot your password?** sends a reset link that works once, for 30 minutes. Otherwise ask a workspace admin or the person who runs the server to set a new password.

**If you invite people** (workspace owner or admin)

1. Open **Settings**, then **Workspace** > **Invitations**, and choose **Invite member**.
2. Enter the **Email**, choose a **Role** (**Member**, **Admin** or **Owner**) and, if you want, a Team to join. Only a workspace owner can invite an admin or an owner.
3. Pass on the **Invitation link**. It is shown only now. The server also mails it, but only when it has e-mail set up; the list shows **E-mail** as **Sent**, **Failed** or **Link only**. In the same list, **New link** replaces a lost link and **Revoke** cancels an invitation; for an invitation to an admin or owner, only a workspace owner can do either.

Joining a workspace does not open every project. A project opens only for people who take part in it; see [Who can do what](#who-can-do-what).

## 2. Talk in channels and threads

Open **Chat** in the left bar. **Channels** and **Direct messages** are listed in the sidebar. **+ New conversation** starts a **New direct message** or a **New group conversation**, and, for workspace owners and admins, a **New channel**. A project's admins make that project's channels with the **+** next to **Chat** in the project's sidebar (**Create channel**).

- **Public and private channels.** A public channel can be read by the people it is open to, but you must **Join** it before you can write (the page says so). A private channel is open only to the people in it. A channel manager adds people from **Add people** in the channel's **Channel settings**.
- **Writing.** Type in the message box and press **Enter** to send, **Shift+Enter** for a new line. Type **@** to mention a person, and use **Attach file** or drop a file onto the page to send a file.
- **Message actions.** Point at a message and open **Message actions**. You can **Copy**, **Create work** (see the next section), **Save for later**, **Mark unread from here** and open the **Thread**. On your own messages you can also **Edit** and see **History**. **Delete** is offered for your own messages and, in a channel you manage, for others'. The quick reactions are 👍 ❤️ 😂 🎉 👀.
- **Threads.** Keep a discussion together by opening a message's **Thread** (or choosing its **Replies** count). Write in **Write a reply** and choose **Send**. **Watch thread** sends later replies to your Inbox.
- **Channel settings** (the gear next to a channel) is where the people who run a channel change visibility, people and incoming webhooks; others do not see the gear. **Leave channel** takes you out of one.
- **Direct messages** go to one person or a small group and need no project or channel.

## 3. Turn a message into a ticket

A ticket is a piece of work in a project. You can make one straight from a message, so the conversation stays attached to the work.

1. Open **Message actions** on the message and choose **Create work**.
2. The **Create ticket** panel opens beside the chat with the text of the message filled in: the first line becomes the **Title** and the whole message the description, as a starting point you can edit. You are the default assignee.
3. If the channel belongs to a project, the ticket is created in that project and channel. Otherwise you are asked to **Choose the Project for the Ticket** first.
4. Edit what you need and choose **Create** (or press **Ctrl+Enter**). A new ticket always starts in the project's initial State. A note in the conversation says that a ticket was created from it.

If the panel says you do not have permission to create tickets in the project, you need edit access in it (see [Who can do what](#who-can-do-what)). Closing the panel keeps what you typed for 14 days in this browser (**Reopen** brings it back); attached files are not kept.

You can also make a ticket from a project: choose **Create ticket** at the top of the project, or **Add ticket** on the **Work** tab (on the Board, at the top of the initial State's column).

## 4. Manage tickets: assignee, State, Board and Table

Open **Projects** and choose a project. A project has the tabs **Overview**, **Work**, **Planning**, **Timeline**, **Docs** and **Chat**. **My projects** lists the projects you take part in and **All projects** lists every project's name (you can open only those you take part in). **My Work**, under **Home**, collects the tickets that are **Assigned** to you, that you **Created** or that you are **Watching**.

On the **Work** tab, **Layout** switches between **Board**, **List** and **Table**. They show the same tickets.

- **Board.** One column per State. **Add ticket** in the initial State's column opens the **Create ticket** panel. Drag a card to another column to change its State, or select the card and press **Alt+←** or **Alt+→**.
- **Table.** One row per ticket with columns such as **Key**, **Title**, **State**, **Assignee**, **Priority**, **Due** and **Sprint**. Use **Display** to choose the columns and density, **Filter** to narrow the list and **Group** to split it. Layout, density and columns are yours only; they do not change a shared view for others.
- **Ticket detail.** Choose a ticket to open its panel. Set the **Assignee** and the **State** there (**Change state**), edit the **Description**, and use the **Comments**, **Activity** and **History** tabs. A ticket can have a parent and subtasks, and can wait on other tickets.
- **Why a move is refused.** A ticket cannot be completed while its subtasks or its predecessors are unfinished (the message tells you which), and a ticket whose parent is archived cannot be reopened until the parent is unarchived.
- **Read-only.** Without edit access in a project, boards and tickets are shown read-only and you cannot create or move tickets.

## 5. Link tickets and documents

Documents live under **Docs** in the left bar and under a project's **Docs** tab. Choose **New document**, give it a **Title** and write. A new document stays a draft in this browser until you save it, and you choose who can see it when you save. A document is saved when you choose **Save**; others do not see changes before that, and if someone saved first your draft is not overwritten (you are told, and your text stays so you can copy it). There is no live co-editing. **Version history** keeps versions you create, and **Sharing** sets who can open a document: everyone in the workspace, the people of a project, or only the people you invite by name.

To link work and knowledge together, in a document or in a ticket's description:

1. Type **/** and choose **Reference a Ticket** or **Reference a document**.
2. In **Choose a Ticket** (or **Choose a document**), search with **Find by name** and pick the item. It appears as a link you can follow.
3. Type **@** to mention a person.

A ticket's panel lists the documents and tickets that mention it under **Referenced by**. A link only opens for people who may open the target; to anyone else it shows as an unavailable item.

## 6. Search

Choose **Search** in the left bar and type words, optionally with filters. The tabs narrow the results to **Messages**, **Tickets**, **Docs**, **Files**, **People**, **Channels & projects** or **Views**; **All** mixes them. You only ever see what you can open.

- Filters are written as `key:value` and combined with the words: `deploy status:done assignee:me`. The keys are `in`, `from`, `assignee`, `project`, `team`, `type`, `status`, `label`, `before`, `after`, `has` and `mentions`; not every key applies to every tab. `status` takes `not-started`, `in-progress`, `done` or `canceled`; dates are `YYYY-MM-DD`. The filter chips under the box cover the common ones. `OR`, `NOT` and grouping are not supported.
- Files are found by their names, not by what is inside them.
- If a search finds nothing, the page lists the conditions that applied and offers **Clear all conditions** or **Search everything**.
- If search is down, the page says **Search is not available right now. Everything else keeps working.** and offers **Try again**; finding people, teams, projects and channels still works. A note that a kind of result is **still being indexed** means it will fill in shortly.

## 7. Inbox, Watch and Later

- **Inbox** collects what needs you: mentions, replies, assignments, reminders and invitations. The filters narrow the list; **Mark read**, **Mark unread** and **Mark all read** keep it tidy, and **Unread** shows only what you have not read.
- **Watch.** Watching a ticket, a thread or a project (**Watch** / **Watching**) sends its ordinary updates, such as comments and State changes, to your Inbox. Mentions, direct messages, replies and assignments reach you whether you watch or not.
- **Later.** **Save for later** on a message or a ticket puts it on **Saved** > **Later**, where you can **Mark handled** and set **Remind me** for a time you pick. A reminder rings once. Handling an item does not complete a ticket. **Saved** > **Favorites** keeps shortcuts to the projects, tickets, channels, DMs, docs, teams and saved views you star; they do not subscribe you to anything, and they do not join or share anything. A starred channel or DM is also listed under **Favorites** in Chat, a starred doc at the top of the docs tree (a project's own docs in that project), a starred team at the top of Teams, and a starred view first in the views bar. The order and folders are set in **Saved**; moving a shortcut up or down there changes only its own folder (or the top level). **Saved** is also reached from **Open Saved** on Home.
- **Notifications** under **Settings** chooses between **Everything (including updates on what you watch)** and **Only what is addressed to me**, sets **Do Not Disturb** hours and lists muted channels. Push notifications to a browser or phone appear there only when the person who runs the server has turned them on.

## 8. Language and personal settings

Open **Settings**. Under **My settings**:

| Page | What it is for |
|---|---|
| **Profile** | Your display name, title, time zone, photo and status. Other members see the display name and your local time. |
| **Account and security** | Your e-mail, password, optional **Two-step verification** (an authenticator app; you get ten single-use recovery codes, shown once) and **Signed-in devices** (you can sign out others). |
| **Notifications** | What reaches your Inbox, quiet hours, muted channels, push. |
| **Display** | **Language** (English or 한국어) and **Theme**. They are saved in this browser, not on your account, so set them again on another browser. The first visit follows the browser's language. The log-in page has the same language button. |
| **Connected apps** | AI apps you connected through MCP. **Disconnect** ends an app's access at once. |

Workspace owners and admins also see **Workspace** (members, Teams, **Invitations**, recovery, MCP and the audit log).

## Who can do what

There are two kinds of role. A **workspace role** (**Owner**, **Admin**, **Member**) decides who administers the workspace. A **project role** (**Admin**, **Editor**, **Viewer**) decides what you can do inside a project. The server checks both on every request; a control you cannot use is hidden or read-only, and something you may not see is reported as not found.

| You want to | You need |
|---|---|
| Invite people, create a project or a Team, see the audit log | Workspace owner or admin (only an owner can invite an admin or owner; a security auditor the workspace names can also read the audit log) |
| Open a project and its tickets, channels and documents | To take part in the project (directly, through a Team, or through a group). A workspace owner or admin is **not** included automatically; ask someone in the project |
| Create and move tickets, edit descriptions | **Editor** or **Admin** in the project. A **Viewer** reads only |
| Create a channel inside a project | Project **Admin** |
| Create a workspace-wide channel | Workspace owner or admin |
| Read or write in a private channel | To be in it; a channel manager adds people |
| Change a channel's settings or webhooks | The channel's manager, the project's admin for a project channel, or a workspace owner or admin for a public workspace-wide channel. A project admin runs a private channel's people without reading it; only the channel's manager makes a private channel public |
| Send direct messages, search, use the Inbox | Any workspace member |

If you cannot find something you expect, the usual causes are that you do not take part in its project, are not in its private channel, or that search is still indexing. Operators and administrators find more in [Operations](operations.md) and [Known limitations](known-limitations.md).
