// The dashboard's map: five stages a change moves through, each with its pages. Every page says
// who it is for (developer, everyone, product), so a person can tell which pages are theirs.
import { Bot, Clock, Code2, Eye, FileClock, FilePen, Hammer, Lightbulb, PackageOpen, Radio, ShieldCheck, Telescope, Users, Webhook, Briefcase, Activity, type LucideIcon } from 'lucide-react';

export type Audience = 'developer' | 'shared' | 'product';

export const AUDIENCE: Record<Audience, { icon: LucideIcon; label: string }> = {
  developer: { icon: Code2, label: 'For developers' },
  shared: { icon: Users, label: 'For everyone' },
  product: { icon: Briefcase, label: 'For product and marketing' },
};

export interface FlowPage {
  id: string;
  label: string;
  icon: LucideIcon;
  audience: Audience;
  href: (site: string) => string;
  /** Path prefixes that belong to this page (drill-downs included). */
  match: string[];
  tip: string;
}

export interface Stage {
  id: 'build' | 'approve' | 'run' | 'learn' | 'observe';
  label: string;
  icon: LucideIcon;
  tip: string;
  pages: FlowPage[];
}

export const STAGES: Stage[] = [
  {
    id: 'build',
    label: 'Build',
    icon: Hammer,
    tip: 'Say what can change on the site',
    pages: [
      { id: 'install', label: 'Install', icon: PackageOpen, audience: 'developer', href: (s) => `/install/?site=${s}`, match: ['/install'], tip: 'Put Pratibimba on the site' },
      { id: 'agent', label: 'Agent', icon: Bot, audience: 'developer', href: (s) => `/agent/?site=${s}`, match: ['/agent'], tip: 'Connect a coding agent over MCP' },
      { id: 'drafts', label: 'Drafts', icon: FilePen, audience: 'product', href: (s) => `/drafts/?site=${s}`, match: ['/drafts'], tip: 'Write a draft and upload images' },
    ],
  },
  {
    id: 'approve',
    label: 'Approve',
    icon: ShieldCheck,
    tip: 'A person lets drafts go live',
    pages: [
      { id: 'review', label: 'Review', icon: ShieldCheck, audience: 'product', href: (s) => `/approvals/?site=${s}`, match: ['/approvals'], tip: 'Approve or reject each draft' },
      { id: 'preview', label: 'Preview', icon: Eye, audience: 'shared', href: (s) => `/preview/?site=${s}`, match: ['/preview'], tip: 'See the real site as different visitors' },
    ],
  },
  {
    id: 'run',
    label: 'Run',
    icon: Radio,
    tip: 'What visitors are being shown now',
    pages: [{ id: 'live', label: 'Live', icon: Activity, audience: 'shared', href: (s) => `/site/?id=${s}`, match: ['/site', '/slot', '/decision'], tip: 'Live numbers, slots and the pause switch' }],
  },
  {
    id: 'learn',
    label: 'Learn',
    icon: Lightbulb,
    tip: 'What the outcomes taught the model',
    pages: [
      { id: 'results', label: 'Results', icon: Lightbulb, audience: 'product', href: (s) => `/results/?site=${s}`, match: ['/results'], tip: 'Lift against the holdout, and learnings' },
      { id: 'time-travel', label: 'Time travel', icon: Clock, audience: 'developer', href: (s) => `/time-travel/?site=${s}`, match: ['/time-travel'], tip: 'Skip a visitor forward in a sandbox copy' },
    ],
  },
  {
    id: 'observe',
    label: 'Observe',
    icon: Telescope,
    tip: 'Who did what, and who was told',
    pages: [
      { id: 'audit', label: 'Audit', icon: FileClock, audience: 'developer', href: (s) => `/audit/?site=${s}`, match: ['/audit'], tip: 'Every agent tool call' },
      { id: 'webhooks', label: 'Webhooks', icon: Webhook, audience: 'developer', href: (s) => `/webhooks/?site=${s}`, match: ['/webhooks'], tip: 'Tell other systems what changed' },
    ],
  },
];

export function locate(path: string): { stage: Stage; page: FlowPage } | null {
  for (const stage of STAGES) for (const page of stage.pages) if (page.match.some((m) => path.startsWith(m) || path.startsWith(`/app${m}`))) return { stage, page };
  return null;
}

/** The stage after this one, for the "next" link at the foot of each page. */
export function nextStage(id: Stage['id']): Stage | null {
  const i = STAGES.findIndex((s) => s.id === id);
  return STAGES[i + 1] ?? null;
}
