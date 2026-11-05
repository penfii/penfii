import { Component, type ReactNode } from 'react'
import { Button } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'

/** What the person sees when a screen cannot be drawn; the page reload starts every screen again. */
export function RenderFailure() {
  const { t } = useI18n()
  return (
    <div className="render-failure" role="alert" data-state="error">
      <p>{t('app.renderError')}</p>
      <Button type="button" onClick={() => window.location.reload()}>{t('app.reload')}</Button>
    </div>
  )
}

/**
 * Keeps a failing screen (a render error, or a page chunk that did not load) from taking the rest of the shell with it:
 * the navigation stays, and going to another screen (`resetKey`) tries again.
 */
export class RenderBoundary extends Component<{ children: ReactNode; resetKey: string }, { failed: boolean }> {
  override state = { failed: false }

  static getDerivedStateFromError() {
    return { failed: true }
  }

  override componentDidUpdate(previous: { resetKey: string }) {
    if (this.state.failed && previous.resetKey !== this.props.resetKey) this.setState({ failed: false })
  }

  override render() {
    return this.state.failed ? <RenderFailure /> : this.props.children
  }
}
