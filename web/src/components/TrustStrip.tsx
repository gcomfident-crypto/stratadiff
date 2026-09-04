import { AlertTriangle, Braces, GitCompareArrows, Layers3, ShieldCheck, Split } from 'lucide-react'
import type { DiffReport } from '../types'
import { compactNumber } from '../lib/format'

interface TrustStripProps {
  report: DiffReport
}

export function TrustStrip({ report }: TrustStripProps) {
  const abstentions = report.ambiguities.filter(({ constraint }) => constraint.kind === 'symbolic_abstention').length
  const stats = [
    { label: 'Replay', value: report.certificate.patch_verified ? 'Verified' : 'Failed', icon: ShieldCheck, tone: 'verified' },
    { label: 'Claims', value: compactNumber(report.relations.length), icon: GitCompareArrows, tone: 'neutral' },
    { label: 'Forced', value: compactNumber(report.summary.model_forced_relations), icon: Braces, tone: 'forced' },
    { label: 'Ambiguous', value: compactNumber(report.summary.ambiguity_groups), icon: AlertTriangle, tone: 'ambiguous' },
    { label: 'Abstained', value: compactNumber(abstentions), icon: Layers3, tone: 'abstained' },
    { label: 'Byte edits', value: compactNumber(report.patch.edits.length), icon: Split, tone: 'bytes' },
  ] as const

  return (
    <section className="trust-strip" aria-label="Report trust summary">
      {stats.map(({ label, value, icon: Icon, tone }) => (
        <div className={`trust-stat tone-${tone}`} key={label}>
          <Icon size={15} strokeWidth={1.8} aria-hidden="true" />
          <span className="trust-label">{label}</span>
          <strong>{value}</strong>
        </div>
      ))}
    </section>
  )
}
