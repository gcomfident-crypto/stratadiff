const CONTROL_NAMES = new Map<number, string>([
  [0x00, 'NUL'],
  [0x01, 'SOH'],
  [0x02, 'STX'],
  [0x03, 'ETX'],
  [0x04, 'EOT'],
  [0x05, 'ENQ'],
  [0x06, 'ACK'],
  [0x07, 'BEL'],
  [0x08, 'BS'],
  [0x09, 'TAB'],
  [0x0a, 'LF'],
  [0x0b, 'VT'],
  [0x0c, 'FF'],
  [0x0d, 'CR'],
  [0x0e, 'SO'],
  [0x0f, 'SI'],
  [0x10, 'DLE'],
  [0x11, 'DC1'],
  [0x12, 'DC2'],
  [0x13, 'DC3'],
  [0x14, 'DC4'],
  [0x15, 'NAK'],
  [0x16, 'SYN'],
  [0x17, 'ETB'],
  [0x18, 'CAN'],
  [0x19, 'EM'],
  [0x1a, 'SUB'],
  [0x1b, 'ESC'],
  [0x1c, 'FS'],
  [0x1d, 'GS'],
  [0x1e, 'RS'],
  [0x1f, 'US'],
  [0x7f, 'DEL'],
  [0x85, 'NEL'],
  [0xad, 'SHY'],
  [0x61c, 'ALM'],
  [0x200b, 'ZWSP'],
  [0x200c, 'ZWNJ'],
  [0x200d, 'ZWJ'],
  [0x200e, 'LRM'],
  [0x200f, 'RLM'],
  [0x2028, 'LS'],
  [0x2029, 'PS'],
  [0x202a, 'LRE'],
  [0x202b, 'RLE'],
  [0x202c, 'PDF'],
  [0x202d, 'LRO'],
  [0x202e, 'RLO'],
  [0x2060, 'WJ'],
  [0x2061, 'FA'],
  [0x2062, 'IT'],
  [0x2063, 'IS'],
  [0x2064, 'IP'],
  [0x2066, 'LRI'],
  [0x2067, 'RLI'],
  [0x2068, 'FSI'],
  [0x2069, 'PDI'],
  [0x27e6, 'TOKEN-OPEN'],
  [0x27e7, 'TOKEN-CLOSE'],
  [0xfeff, 'BOM'],
])

const FORMAT_CHARACTER = /\p{Cf}/u

function codePointLabel(codePoint: number): string {
  const name = CONTROL_NAMES.get(codePoint) ?? 'CONTROL'
  const width = codePoint <= 0xffff ? 4 : 6
  return `⟦${name} U+${codePoint.toString(16).toUpperCase().padStart(width, '0')}⟧`
}

function shouldVisualize(character: string, codePoint: number): boolean {
  return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f) || codePoint === 0x2028 || codePoint === 0x2029 || codePoint === 0x27e6 || codePoint === 0x27e7 || FORMAT_CHARACTER.test(character)
}

function visualize(value: string, preserveLineFeedAndTab: boolean, maxOutputLength: number): { text: string | null; visualized: boolean } {
  let output = ''
  let visualized = false
  for (const character of value) {
    const codePoint = character.codePointAt(0)
    if (codePoint === undefined) throw new Error('A source character has no Unicode code point.')
    let rendered = character
    if (!(preserveLineFeedAndTab && (codePoint === 0x09 || codePoint === 0x0a)) && shouldVisualize(character, codePoint)) {
      rendered = codePointLabel(codePoint)
      visualized = true
    }
    if (output.length + rendered.length > maxOutputLength) return { text: null, visualized }
    output += rendered
  }
  return { text: output, visualized }
}

export function visibleSourceText(value: string, maxOutputLength = Number.MAX_SAFE_INTEGER): { text: string | null; visualized: boolean } {
  return visualize(value, true, maxOutputLength)
}

export function visibleInlineText(value: string): string {
  const result = visualize(value, false, Number.MAX_SAFE_INTEGER).text
  if (result === null) throw new Error('Inline display text exceeded the JavaScript string limit.')
  return result
}
