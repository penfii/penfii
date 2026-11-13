import type { ReactNode } from 'react'

/** Stroke icons for controls that must not depend on an emoji font; decorative, so the control carries the name. */
function Icon({ children, size = 16 }: { children: ReactNode; size?: number }) {
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
      {children}
    </svg>
  )
}

export const PaperclipIcon = () => <Icon><path d="m20 11-8.5 8.5a5 5 0 0 1-7-7L13 4a3.5 3.5 0 0 1 5 5l-8.6 8.6a2 2 0 0 1-3-3L14 7" /></Icon>
export const DownloadIcon = () => <Icon><path d="M12 4v11m0 0-4-4m4 4 4-4M5 20h14" /></Icon>
export const CloseIcon = () => <Icon><path d="m6 6 12 12M18 6 6 18" /></Icon>
export const TrashIcon = () => <Icon><path d="M4 7h16M10 11v6m4-6v6M6 7l1 13h10l1-13M9 7V4h6v3" /></Icon>
export const MoreIcon = () => <Icon><circle cx="5" cy="12" r="1.2" fill="currentColor" /><circle cx="12" cy="12" r="1.2" fill="currentColor" /><circle cx="19" cy="12" r="1.2" fill="currentColor" /></Icon>
export const ChevronLeftIcon = () => <Icon><path d="m15 5-7 7 7 7" /></Icon>
export const ChevronRightIcon = () => <Icon><path d="m9 5 7 7-7 7" /></Icon>
export const GearIcon = () => <Icon><circle cx="12" cy="12" r="3" /><path d="M12 3v2m0 14v2m9-9h-2M5 12H3m15.4-6.4-1.4 1.4M7 17l-1.4 1.4m12.8 0L17 17M7 7 5.6 5.6" /></Icon>
export const FileIcon = () => <Icon size={22}><path d="M7 3h7l5 5v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM14 3v5h5" /></Icon>
export const EyeIcon = () => <Icon><path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6z" /><circle cx="12" cy="12" r="2.5" /></Icon>
export const PinIcon = () => <Icon><path d="M9 4h6l-1 6 3 3H7l3-3zM12 13v7" /></Icon>
export const BookmarkIcon = () => <Icon><path d="M6 4h12v16l-6-4-6 4z" /></Icon>
export const ThreadIcon = () => <Icon><path d="M5 5h14a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H10l-5 4v-4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z" /></Icon>
export const StarIcon = () => <Icon><path d="m12 4 2.4 5 5.6.7-4.1 3.8 1.1 5.5-5-2.8-5 2.8 1.1-5.5L4 9.7 9.6 9z" /></Icon>

export const PlayIcon = () => <Icon><path d="M8 5.5v13l11-6.5z" /></Icon>
export const ExpandIcon = () => <Icon><path d="M14 4h6v6M10 20H4v-6M20 4l-7 7M4 20l7-7" /></Icon>

export const CheckIcon = () => <Icon size={18}><path d="m5 12.5 4.5 4.5L19 7.5" /></Icon>
export const DotIcon = () => <Icon size={18}><circle cx="12" cy="12" r="4.5" fill="currentColor" /></Icon>
export const ClockIcon = () => <Icon size={18}><circle cx="12" cy="12" r="8.5" /><path d="M12 7.5V12l3 2" /></Icon>
export const SunriseIcon = () => <Icon size={18}><path d="M4 18h16M7 18a5 5 0 0 1 10 0M12 6v3M5.6 9.6l1.7 1.7M18.4 9.6l-1.7 1.7" /></Icon>
export const ArchiveIcon = () => <Icon size={18}><path d="M4 5h16v4H4zM5.5 9v9.5a1 1 0 0 0 1 1h11a1 1 0 0 0 1-1V9M10 13h4" /></Icon>
export const UndoIcon = () => <Icon size={18}><path d="M9 7 4.5 11.5 9 16M5 11.5h9a5 5 0 0 1 5 5V18" /></Icon>
