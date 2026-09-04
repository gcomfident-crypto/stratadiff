import { afterEach, describe, expect, it, vi } from 'vitest'
import { sessionFixture } from '../test/fixture'
import { decodeUtf8, fetchSession, getSessionToken } from './session'

describe('session loading', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('requires an explicit viewer token', () => {
    expect(() => getSessionToken('')).toThrow('Missing viewer session token')
    expect(getSessionToken('?token=one-time%20token')).toBe('one-time token')
  })

  it('loads the fixed API contract and preserves source bytes', async () => {
    const payload = sessionFixture()
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.startsWith('/api/session')) return Promise.resolve(new Response(JSON.stringify(payload), { status: 200 }))
      if (url.startsWith('/api/source/before')) return Promise.resolve(new Response(new TextEncoder().encode('const before = 1\n'), { status: 200 }))
      if (url.startsWith('/api/source/after')) return Promise.resolve(new Response(new TextEncoder().encode('const after = 2\n'), { status: 200 }))
      throw new Error(`Unexpected URL: ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    const session = await fetchSession('?token=abc/123')

    expect(fetchMock).toHaveBeenCalledTimes(3)
    expect(fetchMock).toHaveBeenCalledWith('/api/session?token=abc%2F123', expect.objectContaining({ cache: 'no-store' }))
    expect(fetchMock).toHaveBeenCalledWith('/api/source/before?token=abc%2F123', expect.objectContaining({ cache: 'no-store' }))
    expect(fetchMock).toHaveBeenCalledWith('/api/source/after?token=abc%2F123', expect.objectContaining({ cache: 'no-store' }))
    expect(session.decodedBefore.text).toBe('const before = 1\n')
    expect(Array.from(session.decodedAfter.bytes)).toEqual(Array.from(new TextEncoder().encode('const after = 2\n')))
  })

  it('detects invalid UTF-8 without replacement characters', () => {
    expect(decodeUtf8(Uint8Array.from([0xff, 0x00, 0xfe]))).toBeNull()
  })

  it('preserves a UTF-8 BOM so a BOM-only change remains visible', () => {
    expect(decodeUtf8(Uint8Array.from([0xef, 0xbb, 0xbf, 0x61]))).toBe('\ufeffa')
    expect(decodeUtf8(Uint8Array.from([0x61]))).toBe('a')
  })

  it('rejects source bytes that do not match the verified report length', async () => {
    const payload = sessionFixture()
    payload.report.before.byte_len += 1
    vi.stubGlobal('fetch', vi.fn().mockImplementation((url: string) => {
      if (url.startsWith('/api/session')) return Promise.resolve(new Response(JSON.stringify(payload), { status: 200 }))
      const source = url.includes('/before') ? 'const before = 1\n' : 'const after = 2\n'
      return Promise.resolve(new Response(new TextEncoder().encode(source), { status: 200 }))
    }))

    await expect(fetchSession('?token=valid')).rejects.toThrow('lengths do not match')
  })
})
