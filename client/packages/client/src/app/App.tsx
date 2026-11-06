import { Route, RouterProvider, Routes, createBrowserRouter } from 'react-router'
import { QueryClientProvider } from '@tanstack/react-query'
import { lazy, Suspense, useEffect, useLayoutEffect, useRef, type ComponentType, type ReactNode } from 'react'
import { setUnauthorizedHandler } from '@penfii/api-client'
import { useLocation, useNavigate } from 'react-router'
import { I18nProvider, useI18n } from '../i18n/I18nProvider'
import { LoginPage } from '../pages/LoginPage'
import { NotFoundState } from '../ui/NotFoundState'
import { RenderBoundary, RenderFailure } from '../ui/RenderBoundary'
import '../styles.css'
import { queryClient } from './queryClient'
import { OfflineWrites } from './offlineWrites'
import { RequireAuth } from './RequireAuth'
import { rememberReturn } from './returnTo'
import { MediaViewerHost } from '../modules/files/mediaViewer'
import { applyTheme, readTheme } from '../design/theme'
import { WorkspaceShell } from '../shell/WorkspaceShell'
import { MobileShell } from '../mobile/MobileShell'
import { useIsMobile } from '../design/useIsMobile'

/** A page loaded on first visit, so the first screen does not wait for every other page. */
function page<M>(load: () => Promise<M>, name: keyof M) {
  return lazy(async () => ({ default: (await load())[name] as ComponentType }))
}

const InvitePage = page(() => import('../pages/InvitePage'), 'InvitePage')
const AccountPage = page(() => import('../pages/AccountPage'), 'AccountPage')
const SettingsPage = page(() => import('../pages/AccountPage'), 'SettingsPage')
const AdminPage = page(() => import('../pages/AdminPage'), 'AdminPage')
const ConsentPage = page(() => import('../pages/ConsentPage'), 'ConsentPage')
const ChatHomePage = page(() => import('../pages/ChatHomePage'), 'ChatHomePage')
const HomePage = page(() => import('../pages/HomePage'), 'HomePage')
const PendingDmPage = page(() => import('../pages/PendingDmPage'), 'PendingDmPage')
const DocsScreen = page(() => import('../modules/docs/DocsScreen'), 'DocsScreen')
const InboxPage = page(() => import('../modules/attention/InboxPage'), 'InboxPage')
const ProjectsPage = page(() => import('../pages/ProjectsPage'), 'ProjectsPage')
const ProjectChannelPage = page(() => import('../pages/ProjectsPage'), 'ProjectChannelPage')
const MyWorkPage = page(() => import('../modules/personal/WorkPages'), 'MyWorkPage')
const TeamWorkPage = page(() => import('../modules/personal/WorkPages'), 'TeamWorkPage')
const DraftsPage = page(() => import('../modules/personal/DraftsPage'), 'DraftsPage')
const SavedPageRoute = page(() => import('../modules/personal/SavedRoute'), 'SavedPageRoute')
const OrganizationPage = page(() => import('../pages/OrganizationPage'), 'OrganizationPage')
const TeamsPage = page(() => import('../pages/TeamsPage'), 'TeamsPage')
const SearchPage = page(() => import('../modules/search/SearchPage'), 'SearchPage')
const ConversationFrame = page(() => import('../pages/SpacePage'), 'ConversationFrame')
const WorkspacesPage = page(() => import('../pages/WorkspacesPage'), 'WorkspacesPage')

function Loading() {
  const { t } = useI18n()
  return <p className="hint" role="status">{t('loading')}</p>
}


function AuthBridge() {
  const navigate = useNavigate()
  const location = useLocation()
  const pathRef = useRef(location.pathname)
  const searchRef = useRef(location.search)
  pathRef.current = location.pathname
  searchRef.current = location.search
  useEffect(() => {
    setUnauthorizedHandler(() => {
      const path = pathRef.current
      if (path === '/login' || path.startsWith('/invite/')) return
      rememberReturn(path + searchRef.current)
      void navigate('/login', { replace: true })
    })
    return () => setUnauthorizedHandler(null)
  }, [navigate])
  return null
}

function ThemeSync() {
  useLayoutEffect(() => {
    const mode = readTheme()
    applyTheme(mode)
    if (mode !== 'system') return
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const onChange = () => applyTheme('system')
    media.addEventListener('change', onChange)
    return () => media.removeEventListener('change', onChange)
  }, [])
  return null
}

function Shell({ section, children }: { section: 'home' | 'inbox' | 'chat' | 'projects' | 'docs' | 'teams' | 'search' | 'settings'; children: ReactNode }) {
  const location = useLocation()
  const Frame = useIsMobile() ? MobileShell : WorkspaceShell
  return (
    <RequireAuth>
      <Frame section={section}>
        <RenderBoundary resetKey={location.pathname}>
          <Suspense fallback={<Loading />}>{children}</Suspense>
        </RenderBoundary>
      </Frame>
    </RequireAuth>
  )
}

/** Every route of the app. */
function AppRoutes() {
  return (
    <>
      <AuthBridge />
      <Suspense fallback={<Loading />}>
        <Routes>
          <Route path="/login" element={<LoginPage />} />
          <Route path="/invite/:token" element={<InvitePage />} />
          <Route path="/consent" element={<RequireAuth><ConsentPage /></RequireAuth>} />
          <Route path="/" element={<RequireAuth><WorkspacesPage /></RequireAuth>} />
          <Route path="/account" element={<RequireAuth><AccountPage /></RequireAuth>} />
          <Route path="/w/:workspaceId" element={<Shell section="home"><HomePage /></Shell>} />
          <Route path="/w/:workspaceId/inbox" element={<Shell section="inbox"><InboxPage /></Shell>} />
          <Route path="/w/:workspaceId/chat" element={<Shell section="chat"><ChatHomePage /></Shell>} />
          <Route path="/w/:workspaceId/dm/new" element={<Shell section="chat"><PendingDmPage /></Shell>} />
          <Route path="/w/:workspaceId/c/:conversationId" element={<Shell section="chat"><ConversationFrame /></Shell>} />
          <Route path="/w/:workspaceId/c/:conversationId/t/:rootId" element={<Shell section="chat"><ConversationFrame /></Shell>} />
          <Route path="/w/:workspaceId/projects" element={<Shell section="projects"><ProjectsPage /></Shell>} />
          <Route path="/w/:workspaceId/projects/:projectId" element={<Shell section="projects"><ProjectsPage /></Shell>} />
          <Route path="/w/:workspaceId/projects/:projectId/chat/:conversationId" element={<Shell section="projects"><ProjectChannelPage /></Shell>} />
          <Route path="/w/:workspaceId/projects/:projectId/chat/:conversationId/t/:rootId" element={<Shell section="projects"><ProjectChannelPage /></Shell>} />
          <Route path="/w/:workspaceId/projects/:projectId/:view" element={<Shell section="projects"><ProjectsPage /></Shell>} />
          <Route path="/w/:workspaceId/my-work" element={<Shell section="projects"><MyWorkPage /></Shell>} />
          <Route path="/w/:workspaceId/teams/:teamId" element={<Shell section="projects"><TeamWorkPage /></Shell>} />
          <Route path="/w/:workspaceId/saved" element={<Shell section="inbox"><SavedPageRoute /></Shell>} />
          <Route path="/w/:workspaceId/drafts" element={<Shell section="inbox"><DraftsPage /></Shell>} />
          <Route path="/w/:workspaceId/docs" element={<Shell section="docs"><DocsScreen /></Shell>} />
          <Route path="/w/:workspaceId/directory" element={<Shell section="teams"><TeamsPage /></Shell>} />
          <Route path="/w/:workspaceId/directory/members" element={<Shell section="teams"><TeamsPage /></Shell>} />
          <Route path="/w/:workspaceId/directory/teams/:teamId" element={<Shell section="teams"><TeamsPage /></Shell>} />
          <Route path="/w/:workspaceId/search" element={<Shell section="search"><SearchPage /></Shell>} />
          <Route path="/w/:workspaceId/settings" element={<Shell section="settings"><SettingsPage /></Shell>} />
          <Route path="/w/:workspaceId/settings/account" element={<Shell section="settings"><SettingsPage /></Shell>} />
          <Route path="/w/:workspaceId/settings/notifications" element={<Shell section="settings"><SettingsPage /></Shell>} />
          <Route path="/w/:workspaceId/settings/appearance" element={<Shell section="settings"><SettingsPage /></Shell>} />
          <Route path="/w/:workspaceId/settings/apps" element={<Shell section="settings"><SettingsPage /></Shell>} />
          <Route path="/w/:workspaceId/settings/admin" element={<Shell section="settings"><AdminPage /></Shell>} />
          <Route path="/w/:workspaceId/directory/organizations/:organizationId" element={<Shell section="search"><OrganizationPage /></Shell>} />
          <Route path="*" element={<NotFoundState escape />} />
        </Routes>
      </Suspense>
    </>
  )
}

// A data router, so that a page holding unsaved work can ask to be told before any navigation (links,
// buttons, back and forward) leaves it. All routing stays in `AppRoutes`.
const router = createBrowserRouter([{ path: '*', element: <AppRoutes />, errorElement: <RenderFailure /> }])

export function App() {
  return (
    <I18nProvider>
      <ThemeSync />
      <OfflineWrites />
      <MediaViewerHost />
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </I18nProvider>
  )
}
