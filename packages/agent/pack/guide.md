# Working with Pratibimba

Pratibimba personalises marked regions of this site. Each region is a **slot** (for example
`hero`). Each slot has a few versions, called **variants**, and one of them is the
**baseline**: the content the page already renders. For each visitor, Pratibimba picks the
approved variant they are most likely to act on, and it learns from what visitors actually do.

You can read everything Pratibimba knows, and you can submit new variants. **You cannot put a
variant on the site.** Everything you submit is a **draft**. A draft is shown to no visitor
until a person approves it in the Pratibimba dashboard. There is no approve tool, and none is
coming. That is deliberate, and it is the most important fact in this file.

## The connection

The tools come from the MCP server named `pratibimba` at `{{MCP_URL}}`. Its token is read from
the `{{TOKEN_ENV}}` environment variable.

If the `pratibimba` tools are missing or every call fails with 401, stop and tell the user.
They need to create a token (`pratibimba mcp-token <site> "<agent name>"` on the Core host, or
the dashboard's site home), export it as `{{TOKEN_ENV}}`, and restart you. Do not guess what
the tools would have returned. Never write a number you did not read from a tool.

## Every call needs `agent` and `justification`

Every tool needs two arguments, and a call without them is refused:

- `agent`: your name as a product, for example `"Claude Code"` or `"Cursor"`. Use the same name
  for every call in a session.
- `justification`: one line that says why you are making **this** call. A reviewer reads these
  lines next to your draft. A good one names the slot and the reason, for example *"Reading
  hero results before drafting variants for returning visitors."* A poor one says nothing,
  such as *"gathering information"* or *"user asked."*

Also pass `on_behalf_of` with the user's name if you know it. Every call is recorded,
including refused ones, so write each line as if your reviewer will read it. They will.

## How to work

Follow these steps in order. Do not skip ahead to step 4 because the brief looks simple. A
variant written without reading the evidence is a guess, and the reviewer can tell.

### 1. Read before you write

Before you propose anything for a slot, make these calls in this session:

1. `get_site`. This gives the origin, the holdout fraction, and the **forbidden claims**. A
   forbidden claim is a hard limit. Never write one, not even reworded. It also gives
   `facts`, the site's facts.md. Use only figures (prices, percentages, delivery days) that
   appear there. The grounding check flags any other figure on your draft.
2. `list_slots`. This gives the slot's success metric (for example `click`), its selector,
   its current **champion** (the approved variant serving the most traffic), and its variant
   counts by status.
3. `get_results` with `slot`. For each variant you get `counts` (decisions shown, and
   conversions), `share` (its part of the traffic), and `lift` against the holdout as
   `estimate`, `lo` and `hi`.
   - If `lo` is above zero, the variant beats the holdout. If `hi` is below zero, it loses.
   - If `lo` and `hi` straddle zero, or `lift` is null, there is **no evidence yet**. Say so.
     Do not call a variant "winning" because its estimate is a bit higher.
   - A few hundred decisions is not much. Report the sample size along with any claim.
4. `list_variants` with `slot`. Read every variant, not only the approved ones. `rejected`
   variants show what a person refused, and the comment thread shows why. `paused` variants
   were stopped by auto-rollback because they were losing. Treat both as findings: do not
   resubmit the same idea in new words. The comment thread on your own earlier drafts is how a
   reviewer asks you for changes.
5. `list_decisions` for the slot, then `explain_decision` on at least one converted decision
   and one missed decision. This shows which visitor features the bandit is using
   (`returning`, `mobile`, `from_ad`) and how far apart the variants' scores are.

Then tell the user what you found, in a few plain lines: what wins, for whom, how sure the
evidence is, and what has already been tried or rejected. Write variants only after that.

### 2. Find the slot in the code

Search the repo for the slot key:

- Next.js: `<prb.Slot id="<slot>"`. The children are the baseline. The `variants` prop maps
  variant keys to components.
- Edge worker: `<!-- prb:slot="<slot>" -->`.
- Script tag: the slot's `selector` from `list_slots`.

Open the baseline component and every variant component already registered for the slot.
Then read the copy around it: nearby headings, buttons, product names, and the tone of the
rest of the page. That is your material.

### 3. Write the variant from the repo's own material

- **Use what the repo already uses.** Build from the components, class names, design tokens
  and layout of the baseline. Do not bring in a new styling approach. Do not add inline
  styles unless the repo already uses them.
- **Use only true facts.** Every product, price, number, feature, deadline and guarantee must
  come from this repo or from the user. Do not invent discounts, statistics, testimonials,
  urgency or scarcity. If an idea depends on a claim you cannot point to, ask the user rather
  than writing it.
- **Change one idea per variant.** One variant might change the headline's angle, another the
  call to action. Then the results can say which idea worked.
- **Keep the slot's job.** If the metric is `click`, keep one clear primary action. It must
  link to a route that exists in this repo.
- **Next.js: write a real component.** Make it a sibling file next to the baseline, for
  example `components/ReturningHero.tsx`. Register it in the slot's `variants` prop under the
  exact key you will propose: `variants={{ "returning-reorder": <ReturningHero /> }}`.
  Registering an unapproved key is safe. The slot renders only what Pratibimba decides, and
  Pratibimba only decides between approved variants. The file is an ordinary code change, so
  the user reviews and commits it like any other.
- **Write the `html` argument as well.** It is a static HTML fragment of what the component
  renders, with the same text, the same class names and real links. Script-tag and edge
  installs serve it, and the reviewer's diff is computed from it. It must not run code: no
  `<script>`, no `on…=` attributes, no `javascript:` links, and no `<iframe>`, `<object>`,
  `<embed>`, `<base>` or `<meta>`. `<style>` and `<link>` cause a warning, so leave them out.
- **Pick a new key.** Use a lowercase slug that names the idea, such as `returning-reorder` or
  `gift-angle`. Never reuse an existing key. Keys only go forward.
- **Never add variants to the site file** (`site.json` or similar). Applying the site file
  approves what it contains, so that route skips the person who is meant to review your work.

### 4. Propose, one call per variant

Call `propose_variant` once for each candidate, with `slot`, `key` and `html`:

- `justification`: your hypothesis in one line, tied to the evidence you read. For example:
  *"Returning visitors convert 2x on the champion's reorder link (n=1,240); this variant leads
  with reorder."*
- `note`: written for the reviewer. Say what changes compared with the champion, which
  evidence led to it, which file implements it, and what you are unsure about.

Then read the `checks` in the result:

- If any check is `fail`, the draft **cannot be approved** as it is. Fix the problem and
  propose again under a new key. In the note, say which draft the new one replaces.
- If a check is `warn`, tell the user what it said.

To revise a draft after reviewer comments, do the same: propose under a new key, and say in
the note which comment it answers.

### 5. Report what actually happened

End with a summary like this one:

> Submitted 2 drafts for `hero`: `returning-reorder` and `gift-angle`. Both passed the
> constraint checks. They are waiting for approval in the Pratibimba dashboard, and nothing
> has changed on the live site yet. I added `components/ReturningHero.tsx` and
> `components/GiftAngleHero.tsx`, and registered both on the hero slot. Those files are
> uncommitted.

## Never claim a draft is live

A person approves. You do not, and you cannot.

- Never say or suggest that a draft is live, launched, published, deployed, running, in an
  experiment, being tested, or receiving traffic. Not in your summary, not in a commit
  message, not in a note.
- Even after approval, the bandit decides how much traffic a variant gets. Do not promise any
  exposure or any result.
- If the user asks you to approve, publish or "turn on" a draft, tell them that it happens in
  the Pratibimba dashboard, and that only a person can do it. Do not look for a way around
  this. Do not ask for the admin token. Do not call the `/api` routes. Do not edit the site
  file to make it happen.

## The simulator is a sandbox

`simulate` and `fast_forward_visitor` run only in the site's sandbox, a copy of the site with
its own clock and its own models. Real visitors never see it, and nothing it learns reaches the
live site.

- Use `simulate` to check a hypothesis against a planted truth before you propose, for example
  "if returning visitors prefer reorder, does the policy find it in 14 days?" Say in your
  summary that the numbers came from a simulation with the rates you chose.
- `fast_forward_visitor` moves the sandbox's one clock for every simulated visitor, not just the
  one you name.
- Never quote a simulated number as a result of the live site.

## Other limits

- Decisions describe anonymous visitors through a few behaviour and situation features only
  (returning, mobile, from an ad, local time, stated tastes). Do not try to identify
  or profile anyone from them.
- Do not propose for a slot whose results and variant history you have not read in this
  session, even when the brief says "just write a few."
- If the evidence suggests no new variant is needed (for example, the slot has almost no
  traffic, or the champion is still unproven), say that. "Wait for more data" is a valid
  answer.
