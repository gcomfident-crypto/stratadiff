import { describe, expect, it } from 'vitest'
import { relationMatchesQuery } from './evidenceSearch'
import { sessionFixture } from '../test/fixture'

describe('relation search', () => {
  it('indexes the relation number, node ids, and byte offsets', () => {
    const template = sessionFixture().report.relations[0]
    if (template === undefined) throw new Error('Fixture relation is missing.')
    const relation = {
      ...template,
      before: {
        ...template.before,
        id: 77,
        span: { ...template.before.span, start_byte: 101, end_byte: 109 },
      },
      after: {
        ...template.after,
        id: 88,
        span: { ...template.after.span, start_byte: 201, end_byte: 210 },
      },
    }

    expect(relationMatchesQuery(relation, 122, 'R123')).toBe(true)
    expect(relationMatchesQuery(relation, 122, '#77')).toBe(true)
    expect(relationMatchesQuery(relation, 122, 'after node 88')).toBe(true)
    expect(relationMatchesQuery(relation, 122, 'before bytes 101-109')).toBe(true)
    expect(relationMatchesQuery(relation, 122, '201-210')).toBe(true)
    expect(relationMatchesQuery(relation, 121, 'R123')).toBe(false)
  })
})
