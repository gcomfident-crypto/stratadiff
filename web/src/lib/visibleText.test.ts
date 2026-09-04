import { describe, expect, it } from 'vitest'
import { visibleInlineText, visibleSourceText } from './visibleText'

describe('visible text', () => {
  it('preserves source line feeds and tabs while labeling hidden controls', () => {
    expect(visibleSourceText('\ufefflet value\t= "a\0b"\n// \u202e').text).toBe(
      '⟦BOM U+FEFF⟧let value\t= "a⟦NUL U+0000⟧b"\n// ⟦RLO U+202E⟧',
    )
  })

  it('labels all controls in inline text', () => {
    expect(visibleInlineText('dir\nname\t\x1b\x7f\u0085')).toBe(
      'dir⟦LF U+000A⟧name⟦TAB U+0009⟧⟦ESC U+001B⟧⟦DEL U+007F⟧⟦NEL U+0085⟧',
    )
  })

  it('reports substitutions and applies an output limit', () => {
    expect(visibleSourceText('safe\tline\n').visualized).toBe(false)
    expect(visibleSourceText('unsafe\u2066name').visualized).toBe(true)
    expect(visibleSourceText('\0'.repeat(100), 32).text).toBeNull()
  })

  it('cannot collide with source text that resembles a generated token', () => {
    const control = visibleSourceText('\0').text
    const literal = visibleSourceText('⟦NUL U+0000⟧').text
    expect(control).not.toBe(literal)
    expect(literal).toContain('TOKEN-OPEN')
  })
})
