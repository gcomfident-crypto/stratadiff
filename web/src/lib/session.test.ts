import { afterEach, describe, expect, it, vi } from 'vitest'
import { repositoryBaseDriftSessionFixture, repositorySessionFixture, sessionFixture } from '../test/fixture'
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
    expect(session.kind).toBe('file_diff')
    if (session.kind !== 'file_diff') throw new Error('Expected a file diff session.')
    expect(session.decodedBefore.text).toBe('const before = 1\n')
    expect(Array.from(session.decodedAfter.bytes)).toEqual(Array.from(new TextEncoder().encode('const after = 2\n')))
  })

  it('loads a repository review without prefetching every file', async () => {
    const payload = repositorySessionFixture()
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(payload), { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)

    const session = await fetchSession('?token=repository-token')

    expect(session.kind).toBe('repository_review')
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(fetchMock).toHaveBeenCalledWith('/api/session?token=repository-token', expect.objectContaining({ cache: 'no-store' }))
  })

  it('rejects a repository delta with the wrong artifact contract', async () => {
    const payload = repositorySessionFixture()
    payload.resume_delta.schema = 'https://example.test/review-delta-v1.schema.json' as typeof payload.resume_delta.schema
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(payload), { status: 200 })))

    await expect(fetchSession('?token=repository-token')).rejects.toThrow('inconsistent checkpoint delta metadata')
  })

  it('requires fallback reasons exactly on conservative delta entries', async () => {
    const exactPayload = repositorySessionFixture()
    const exactEntry = exactPayload.resume_delta.entries[0]
    if (exactEntry === undefined) throw new Error('Missing exact delta fixture entry.')
    Object.assign(exactEntry, { fallback_reason: 'unsupported_change' })
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(exactPayload), { status: 200 })))
    await expect(fetchSession('?token=repository-token')).rejects.toThrow('inconsistent checkpoint delta fallback evidence')

    const fallbackPayload = repositorySessionFixture()
    const fallbackEntry = fallbackPayload.resume_delta.entries[0]
    if (fallbackEntry === undefined || fallbackEntry.before_source.kind !== 'git_object') throw new Error('Missing Git-backed delta fixture entry.')
    Object.assign(fallbackEntry, { baseline_basis: 'current_base_fallback' })
    fallbackEntry.before_source.commit = fallbackPayload.review.base_commit
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(fallbackPayload), { status: 200 })))
    await expect(fetchSession('?token=repository-token')).rejects.toThrow('inconsistent checkpoint delta fallback evidence')
  })

  it('binds base-drift context to the exact old and current merge bases', async () => {
    const payload = repositoryBaseDriftSessionFixture()
    if (payload.base_drift.status !== 'available') throw new Error('Missing base-drift fixture.')
    payload.base_drift.delta.head_commit = payload.review.head_commit
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(payload), { status: 200 })))

    await expect(fetchSession('?token=repository-token')).rejects.toThrow('inconsistent base-drift metadata')
  })

  it.each(['source kind', 'object id', 'byte length'] as const)(
    'binds each base-drift %s to its review-file metadata',
    async (mismatch) => {
      const payload = repositoryBaseDriftSessionFixture()
      if (payload.base_drift.status !== 'available') throw new Error('Missing base-drift fixture.')
      const entry = payload.base_drift.delta.entries[0]
      if (entry === undefined) throw new Error('Missing base-drift entry.')
      if (mismatch === 'source kind') {
        entry.before_source = { kind: 'empty' }
      } else if (mismatch === 'object id') {
        if (entry.before_source.kind !== 'git_object') throw new Error('Missing before Git source.')
        entry.before_source.object_id = 'f'.repeat(64)
      } else {
        if (entry.after_source.kind !== 'git_object') throw new Error('Missing after Git source.')
        entry.after_source.byte_len = (entry.file.after_bytes ?? 0) + 1
      }
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(payload), { status: 200 })))

      await expect(fetchSession('?token=repository-token')).rejects.toThrow('invalid base-drift file')
    },
  )

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
