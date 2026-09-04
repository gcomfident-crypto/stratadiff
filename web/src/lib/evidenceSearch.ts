import type { Relation } from '../types'

export function includesQuery(values: Array<string | number | undefined>, query: string): boolean {
  return values.join(' ').toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())
}

export function relationSearchValues(relation: Relation, index: number): Array<string | number | undefined> {
  const before = relation.before
  const after = relation.after

  return [
    `R${index + 1}`,
    `relation ${index + 1}`,
    before.kind,
    before.field,
    `before node ${before.id}`,
    `before #${before.id}`,
    `#${before.id}`,
    `before bytes ${before.span.start_byte}-${before.span.end_byte}`,
    before.span.start_byte,
    before.span.end_byte,
    after.kind,
    after.field,
    `after node ${after.id}`,
    `after #${after.id}`,
    `#${after.id}`,
    `after bytes ${after.span.start_byte}-${after.span.end_byte}`,
    after.span.start_byte,
    after.span.end_byte,
    relation.predicate,
    relation.correspondence,
    ...relation.evidence,
  ]
}

export function relationMatchesQuery(relation: Relation, index: number, query: string): boolean {
  return relationMatchesNormalizedQuery(relation, index, query.trim().toLocaleLowerCase())
}

export function relationMatchesNormalizedQuery(relation: Relation, index: number, normalizedQuery: string): boolean {
  return relationSearchValues(relation, index).join(' ').toLocaleLowerCase().includes(normalizedQuery)
}
