import { describe, expect, it } from 'vitest'
import { base64DecodedLength, base64Preview, editAfterRange, editAfterRanges } from './format'
import type { ByteEdit } from '../types'

describe('certified replay ranges', () => {
  const edits: ByteEdit[] = [
    { old_start: 2, old_end: 4, replacement_base64: '5Lit5paH' },
    { old_start: 7, old_end: 9, replacement_base64: 'WA==' },
    { old_start: 10, old_end: 10, replacement_base64: 'IQ==' },
  ]

  it('accounts for all preceding length changes in one linear pass', () => {
    expect(editAfterRanges(edits)).toEqual([[2, 8], [11, 12], [13, 14]])
    expect(editAfterRange(edits, 2)).toEqual([13, 14])
  })

  it('fails clearly for a missing edit', () => {
    expect(() => editAfterRange(edits, 3)).toThrow('Byte edit 3 is missing.')
  })

  it('computes replacement lengths without decoding the payload', () => {
    expect(base64DecodedLength('')).toBe(0)
    expect(base64DecodedLength('YQ==')).toBe(1)
    expect(base64DecodedLength('YWI=')).toBe(2)
    expect(base64DecodedLength('YWJj')).toBe(3)
    expect(() => base64DecodedLength('abc')).toThrow('invalid length')
  })

  it('keeps short Base64 intact and caps large DOM previews', () => {
    expect(base64Preview('YQ==')).toBe('YQ==')
    expect(base64Preview('YWJjZGVm', 4)).toBe('YWJj… (8 Base64 characters)')
  })
})
