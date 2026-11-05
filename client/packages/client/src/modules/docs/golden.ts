/**
 * Cases of the Rust/BlockNote golden fixtures. `input` is canonical DocContent v1
 * blocks; the fixture file records what BlockNote makes of them and what the Rust seeder makes of them.
 */
const U1 = '00000000-0000-0000-0000-0000000000a1'
const U2 = '00000000-0000-0000-0000-0000000000a2'
const F1 = '00000000-0000-0000-0000-0000000000f1'

const t = (text: string, styles: Record<string, unknown> = {}) => ({ type: 'text', text, styles })
const p = (id: string, content: unknown[], children: unknown[] = []) => ({ id, type: 'paragraph', props: {}, content, children })

export const GOLDEN_CASES: Array<{ name: string; blocks: unknown[] }> = [
  { name: 'empty page', blocks: [p('e1', [])] },
  { name: 'korean paragraph', blocks: [p('k1', [t('안녕하세요, 반갑습니다. 배포 절차를 검토합니다.')])] },
  {
    name: 'every style',
    blocks: [
      p('s1', [
        t('굵게', { bold: true }),
        t('기울임', { italic: true }),
        t('밑줄', { underline: true }),
        t('취소선', { strike: true }),
        t('코드', { code: true }),
        t('색', { textColor: 'red' }),
        t('배경', { backgroundColor: 'yellow' }),
        t('모두', { bold: true, italic: true, underline: true, strike: true, code: true, textColor: 'blue', backgroundColor: 'green' }),
        t('일반'),
      ]),
    ],
  },
  {
    name: 'links',
    blocks: [
      p('l1', [
        t('앞 '),
        { type: 'link', href: 'https://example.com/a?b=c', content: [t('링크'), t(' 굵게', { bold: true })] },
        t(' 중간 '),
        { type: 'link', href: '/w/ws/docs?doc=1', content: [t('상대')] },
        { type: 'link', href: 'mailto:a@example.com', content: [t('메일')] },
      ]),
    ],
  },
  {
    name: 'headings and lists',
    blocks: [
      { id: 'h1', type: 'heading', props: { level: 1 }, content: [t('제목 1')], children: [] },
      { id: 'h2', type: 'heading', props: { level: 2 }, content: [t('제목 2')], children: [] },
      { id: 'h3', type: 'heading', props: { level: 3 }, content: [t('제목 3')], children: [] },
      { id: 'li1', type: 'bulletListItem', props: {}, content: [t('불릿')], children: [] },
      { id: 'li2', type: 'numberedListItem', props: {}, content: [t('번호')], children: [] },
      { id: 'li3', type: 'checkListItem', props: { checked: true }, content: [t('완료')], children: [] },
      { id: 'li4', type: 'checkListItem', props: { checked: false }, content: [t('미완')], children: [] },
      { id: 'q1', type: 'quote', props: {}, content: [t('인용')], children: [] },
    ],
  },
  {
    name: 'nested blocks',
    blocks: [
      p('n1', [t('부모')], [
        { id: 'n2', type: 'bulletListItem', props: {}, content: [t('자식')], children: [
          { id: 'n3', type: 'checkListItem', props: { checked: true }, content: [t('손자')], children: [
            p('n4', [t('증손자')]),
          ] },
        ] },
        p('n5', [t('둘째 자식')]),
      ]),
      p('n6', [t('형제')]),
    ],
  },
  {
    name: 'code block',
    blocks: [
      { id: 'c1', type: 'codeBlock', props: { language: 'bash' }, content: [t('kubectl apply -f deploy.yaml\necho "배포 완료"')], children: [] },
      { id: 'c2', type: 'codeBlock', props: { language: 'text' }, content: [], children: [] },
    ],
  },
  {
    name: 'table',
    blocks: [
      {
        id: 'tb1', type: 'table', props: {},
        content: { type: 'tableContent', rows: [
          { cells: [[t('이름', { bold: true })], [t('상태', { bold: true })], [t('담당')]] },
          { cells: [[t('삼성전자')], [t('계약 완료', { textColor: 'green' })], [{ type: 'link', href: 'https://example.com', content: [t('김민수')] }]] },
        ] },
        children: [],
      },
    ],
  },
  {
    name: 'image and file',
    blocks: [
      { id: 'i1', type: 'image', props: { url: `/v1/files/${F1}/content`, name: 'shot.png', caption: '스크린샷', showPreview: true, previewWidth: 320 }, children: [] },
      { id: 'i2', type: 'image', props: { url: `/v1/files/${F1}/content`, name: '', caption: '', showPreview: false }, children: [] },
      { id: 'f1', type: 'file', props: { url: `/v1/files/${F1}/content`, name: '보고서.pdf', caption: '' }, children: [] },
    ],
  },
  {
    name: 'divider',
    blocks: [p('d0', [t('위')]), { id: 'dv', type: 'divider', props: {}, children: [] }, p('d1', [t('아래')])],
  },
  {
    name: 'references and mentions',
    blocks: [
      p('r1', [
        t('티켓 '),
        { type: 'reference', props: { kind: 'TICKET', id: U1 } },
        t(' 와 문서 '),
        { type: 'reference', props: { kind: 'DOC', id: U2 } },
        { type: 'reference', props: { kind: 'RECORD', id: U1 } },
        { type: 'mention', props: { userId: U2 } },
        t(' 확인', { bold: true }),
      ]),
      p('r2', [{ type: 'mention', props: { userId: U1 } }]),
    ],
  },
  {
    name: 'unicode and spacing',
    blocks: [
      p('u1', [t('이모지 😀👍🏽 와 결합문자 é  두 칸 공백 ')]),
      p('u2', [t(' 앞뒤 공백 ')]),
      p('u3', [t('줄바꿈\n두번째 줄')]),
    ],
  },
]
