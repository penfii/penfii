const LABELS: Record<string, string> = {
  plaintext: 'Plain text',
  text: 'Plain text',
  txt: 'Plain text',
  plain: 'Plain text',
  javascript: 'JavaScript',
  js: 'JavaScript',
  typescript: 'TypeScript',
  ts: 'TypeScript',
  jsx: 'JSX',
  tsx: 'TSX',
  html: 'HTML',
  css: 'CSS',
  json: 'JSON',
  bash: 'Bash',
  shell: 'Shell',
  sh: 'Shell',
  zsh: 'Shell',
  python: 'Python',
  py: 'Python',
  rust: 'Rust',
  rs: 'Rust',
  sql: 'SQL',
  yaml: 'YAML',
  yml: 'YAML',
  markdown: 'Markdown',
  md: 'Markdown',
}

export function codeLanguageLabel(language: string | null | undefined): string | null {
  const raw = language?.trim() ?? ''
  if (raw.length === 0) return null
  return LABELS[raw.toLowerCase()] ?? raw
}
