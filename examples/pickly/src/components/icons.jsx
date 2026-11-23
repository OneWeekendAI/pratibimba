/*
 * Pickly icon set — hand-authored, one drawn style.
 * 24x24 viewBox, currentColor, ~1.7 ink stroke with round caps. Paths are
 * deliberately a touch off-true (lines bow, ends overshoot) so they read as
 * ink on paper, not a stock set. No icon libraries anywhere in the app.
 *
 * Every icon: <Name size={20} title="optional" className strokeWidth />
 *   title given  -> role="img" + <title>  (meaningful icon)
 *   no title     -> aria-hidden            (decorative, next to text)
 *
 * Page agents: append new icons ONLY at the bottom, under a comment naming
 * your slice, and only if essential.
 */

function Ink({ size = 20, title, className, strokeWidth = 1.7, children, style, ...rest }) {
  const a11y = title ? { role: 'img' } : { 'aria-hidden': true, focusable: 'false' };
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={['ink-icon', className].filter(Boolean).join(' ')}
      style={style}
      {...a11y}
      {...rest}
    >
      {title ? <title>{title}</title> : null}
      {children}
    </svg>
  );
}

/* A tiny teak shelf holding three jars: the Shop nav icon. */
export function MiniShelf(p) {
  return (
    <Ink {...p}>
      <path d="M2.6 18.2c6.2-.35 12.6-.3 18.9.1" />
      <path d="M4.1 18.4l-.3 2.6M19.9 18.5l.35 2.5" />
      <path d="M4.6 17.9v-5.1c0-.9.6-1.4 1.5-1.4h1.9c.9 0 1.4.5 1.4 1.4v5.1" />
      <path d="M4.9 11.3c.1-.9.4-1.3 1-1.35l2.4-.05c.6.05.8.5.9 1.35" />
      <path d="M10.6 17.9V9.6c0-.8.5-1.3 1.3-1.3h1.6c.9 0 1.3.5 1.3 1.3v8.3" />
      <path d="M10.9 8.2c.15-.85.5-1.2 1.1-1.2h1.4c.6 0 .9.4 1 1.2" />
      <path d="M16.4 17.9v-3.6c0-.8.45-1.2 1.2-1.2h1.3c.8 0 1.2.4 1.2 1.2v3.6" />
      <path d="M5.4 14.6c1-.2 2-.15 3 .05M11.3 12.5c.9-.15 1.7-.1 2.5.05" opacity=".55" />
    </Ink>
  );
}

export function Ladle(p) {
  return (
    <Ink {...p}>
      <path d="M20.6 3.2c-2.6 2.9-5.4 5.8-8.6 9" />
      <path d="M12.3 11.7c-.9-.6-2.2-.7-3.7-.2-2.3.8-4.4 2.9-4.9 5-.5 2.2.9 3.7 3.1 3.4 2.4-.35 4.6-2.4 5.4-4.6.5-1.3.5-2.6.1-3.6z" />
      <path d="M5.6 16.2c.9.3 2.1.2 3.3-.5" opacity=".55" />
    </Ink>
  );
}

export function Jar(p) {
  return (
    <Ink {...p}>
      <path d="M7.3 4.4c3.1-.3 6.3-.3 9.4.05" />
      <path d="M7 4.5c-.15.9-.1 1.7.15 2.4M17 4.5c.15.9.1 1.7-.15 2.35" />
      <path d="M7.2 6.9C5.3 7.6 4.7 9 4.7 11v6.6c0 1.6 1 2.6 2.7 2.7 3 .2 6.2.2 9.2 0 1.7-.1 2.7-1.1 2.7-2.7V11c0-2-.6-3.4-2.5-4.1" />
      <path d="M7.4 6.9c3-.25 6.1-.25 9.2 0" />
      <path d="M7.6 11.2v5.3" opacity=".5" />
      <path d="M9.6 12.6c1.6-.2 3.2-.2 4.8 0l-.2 2.9c-1.5.15-3 .15-4.4 0z" />
    </Ink>
  );
}

export function Lid(p) {
  return (
    <Ink {...p}>
      <path d="M4.2 13.8c-.4-2.6 1.7-4.4 7.8-4.5 6.1-.05 8.3 1.8 7.8 4.5" />
      <path d="M4.2 13.8c.2 1.3.6 2.1 1 2.6 4.3.6 9 .6 13.5 0 .5-.5.9-1.3 1.1-2.6" />
      <path d="M4.6 14.1c4.9.6 9.8.6 14.8 0" opacity=".55" />
      <path d="M10.6 9.3c.1-.9.6-1.4 1.4-1.4s1.3.5 1.4 1.4" />
    </Ink>
  );
}

export function Chilli(p) {
  return (
    <Ink {...p}>
      <path d="M8.3 6.9c-2.3 2.4-3.6 6-3.5 9.6.05 2.2 1 3.6 2.4 3.9 3.1.6 8.7-2.4 11.9-6.6" />
      <path d="M8.3 6.9c2.2-.4 3.9 1 4.2 3.3.3 1.6 1.4 2.6 3.2 3.1 1.1.3 2-.05 2.4-.6" />
      <path d="M8.4 6.8c-.3-1.6.4-3 1.9-3.6M10.3 3.2c.7-.3 1.4-.25 2 .1" />
      <path d="M7 15.4c.4-2 1.2-3.7 2.3-4.9" opacity=".5" />
    </Ink>
  );
}

/* Veg: a curry leaf. */
export function Leaf(p) {
  return (
    <Ink {...p}>
      <path d="M4.3 19.8C4 12.6 8.6 5.3 19.6 4.2c.7 9.6-5 15.2-12.3 15.4" />
      <path d="M3.4 20.7c3.6-4.2 7.2-7.9 11.4-11.6" />
      <path d="M8.6 15.6l-.3-3.2M11.2 13.1l-.1-3.2M13.6 10.9l3.2.1M10.7 16.1l3.3.2" opacity=".6" />
    </Ink>
  );
}

/* Non-veg: a drumstick. */
export function Drumstick(p) {
  return (
    <Ink {...p}>
      <path d="M12.9 13.4c-1.6 1.7-4.6 1.9-6.6.2C4 11.6 3.6 8 5.6 5.7 7.7 3.4 11.6 3.5 13.5 5.9c1.5 1.9 1.6 4.6.6 6.3" fill="currentColor" fillOpacity=".18" />
      <path d="M12.8 13.3l4 4.1" strokeWidth="2.2" />
      <circle cx="18.6" cy="17.1" r="1.45" />
      <circle cx="16.7" cy="19.2" r="1.45" />
      <path d="M7.4 6.6c.9-.9 2.2-1.2 3.4-.8" opacity=".55" />
    </Ink>
  );
}

export function Fish(p) {
  return (
    <Ink {...p}>
      <path d="M3.4 12.1c2.5-3.6 6-5.2 9.4-4.8 2.5.3 4.4 1.7 5.6 4.6-1.3 2.8-3.3 4.2-5.9 4.5-3.4.3-6.7-1-9.1-4.3z" />
      <path d="M18.3 11.9c.9-1.2 1.9-2.1 3-2.6-.4 1.7-.4 3.3.1 5-1.2-.5-2.2-1.3-3.1-2.4" />
      <circle cx="7.4" cy="11.3" r=".6" fill="currentColor" stroke="none" />
      <path d="M10.6 9.6c.8 1.5.8 3.3 0 4.9M13.2 9.3c.6 1.8.6 3.6 0 5.4" opacity=".5" />
    </Ink>
  );
}

/* A newspaper parcel tied with twine. */
export function Parcel(p) {
  return (
    <Ink {...p}>
      <path d="M3.6 8.1c5.5-1.2 11.2-1.3 16.9-.2l-.4 10.6c-5.4 1-10.8 1-16.1.1z" />
      <path d="M3.6 8.1c2.4-1.9 5-3 8-3.2 3-.2 6 .8 8.9 3" />
      <path d="M12 5.2c.2 4.6.2 9.4-.1 14.3" />
      <path d="M3.9 13.2c5.4-.6 10.8-.6 16.2-.05" />
      <path d="M12 13.1c-1.6-1.6-3.6-1.8-3.8-.6-.2 1 1.9 1.1 3.8.6zM12 13.1c1.5-1.7 3.5-2 3.8-.8.2 1.1-1.9 1.2-3.8.8z" />
      <path d="M5.6 10.3h3M5.4 16.2h2.4M15.4 16.1h3" opacity=".45" />
    </Ink>
  );
}

/* A loop of twine with a bow. */
export function Twine(p) {
  return (
    <Ink {...p}>
      <path d="M12 12.1C9.2 9 5.2 8.4 4.6 10.6c-.6 2.3 3.6 2.8 7.4 1.5z" />
      <path d="M12 12.1c2.7-3.2 6.8-3.9 7.4-1.7.6 2.3-3.6 2.9-7.4 1.7z" />
      <path d="M11.6 12.3c-1.2 2.4-3 4.6-5.4 6.4M12.3 12.3c1.4 2.3 3.2 4.3 5.7 5.8" />
      <path d="M2.7 12.6c.8-.1 1.4-.3 1.9-.6M19.4 11.8c.6.3 1.2.5 1.9.5" opacity=".55" />
    </Ink>
  );
}

/* A round postmark with wavy cancel lines. */
export function Postmark(p) {
  return (
    <Ink {...p}>
      <path d="M12.2 4.1c4.4.1 7.6 3.5 7.5 7.9-.1 4.3-3.6 7.6-7.9 7.5-4.3-.1-7.5-3.6-7.4-7.9.1-4.2 3.6-7.6 7.8-7.5z" />
      <path d="M12.1 6.8c2.9.05 5.1 2.4 5 5.3-.05 2.8-2.4 5-5.2 4.9-2.8-.05-5-2.4-4.9-5.2.05-2.8 2.4-5.1 5.1-5z" opacity=".6" />
      <path d="M1.8 9.6c1.4-.9 2.6.8 4 0M18.4 9.6c1.3-.8 2.5.8 3.8-.1M1.8 14.3c1.4-.9 2.6.8 4 0M18.4 14.3c1.3-.8 2.5.8 3.8-.1" />
      <path d="M10.2 12.2h3.6" />
    </Ink>
  );
}

/* A hand-drawn arrow with a little loop in its tail. Points right; rotate with CSS. */
export function ArrowHand(p) {
  return (
    <Ink {...p}>
      <path d="M2.8 14.8c2.4-1.1 4.3-1.4 5.4-.6.9.7.6 2-.5 2.1-1.2.1-1.4-1.4-.3-2.6 2.6-2.8 7.7-3.6 13.3-2.3" />
      <path d="M16.6 8.6c1.6.9 3.1 1.9 4.4 3-1.5.7-3 1.6-4.3 2.9" />
    </Ink>
  );
}

export function Plus(p) {
  return (
    <Ink {...p}>
      <path d="M12.2 4.6c-.25 5-.25 9.9 0 14.8" />
      <path d="M4.7 12.1c4.9-.3 9.8-.25 14.6.1" />
    </Ink>
  );
}

export function Minus(p) {
  return (
    <Ink {...p}>
      <path d="M4.7 12.2c4.9-.35 9.8-.3 14.6.1" />
    </Ink>
  );
}

export function Close(p) {
  return (
    <Ink {...p}>
      <path d="M5.6 5.4c4.4 4.2 8.6 8.6 12.8 13.2" />
      <path d="M18.3 5.6c-4.5 4.2-8.6 8.6-12.6 13" />
    </Ink>
  );
}

export function Back(p) {
  return (
    <Ink {...p}>
      <path d="M20.2 12.4c-5.1-.5-10.3-.5-15.6-.1" />
      <path d="M10.2 6.4C8.3 8.5 6.4 10.4 4.4 12.2c2 1.9 3.9 3.8 5.8 5.8" />
    </Ink>
  );
}

/* The postman's bicycle with a parcel on the carrier (our truck). */
export function Cycle(p) {
  return (
    <Ink {...p}>
      <path d="M6.1 12.6c1.9 0 3.4 1.6 3.4 3.5 0 1.9-1.6 3.4-3.5 3.4-1.9-.05-3.4-1.6-3.3-3.5 0-1.9 1.5-3.4 3.4-3.4z" />
      <path d="M18 12.6c1.9.05 3.4 1.6 3.3 3.5 0 1.9-1.6 3.4-3.5 3.3-1.9 0-3.3-1.6-3.3-3.4 0-1.9 1.6-3.5 3.5-3.4z" />
      <path d="M6 16.1l3.6-6.3 4.9.1 3.5 6.2M9.6 9.8l2.6 6.3h-6.1M14.4 9.9l.9-3.2 2.1-.2" />
      <path d="M8.4 8.2c.9-.1 1.8-.1 2.7 0" />
      <path d="M2.9 8.9l4.3-.2.1 3H3z" opacity=".75" />
    </Ink>
  );
}
export { Cycle as PostCycle };

export function Mail(p) {
  return (
    <Ink {...p}>
      <path d="M3.4 6.4c5.8-.5 11.5-.5 17.3.05l-.3 11.5c-5.6.4-11.2.4-16.7-.05z" />
      <path d="M3.6 6.6c2.6 2.4 5.3 4.4 8.3 6 3-1.6 5.8-3.6 8.6-6" />
      <path d="M3.8 17.7c2-2 4-3.7 6.1-5.2M20.3 17.8c-2-2-4-3.7-6.1-5.2" opacity=".55" />
    </Ink>
  );
}

/* An ink tick: a short stroke, then a long confident flick. */
export function Check(p) {
  return (
    <Ink {...p}>
      <path d="M4.3 12.9c1.5 1.3 2.9 2.8 4.1 4.7 2.6-5.3 6.3-9.6 11.4-12.9" />
      <path d="M8.5 17.4c.4-.9.8-1.7 1.3-2.5" opacity=".5" />
    </Ink>
  );
}

/* Filled chilli for spice meters (same silhouette as Chilli, solid). */
export function ChilliSolid({ fill = 'currentColor', ...p }) {
  return (
    <Ink {...p}>
      <path
        d="M8.3 6.9c-2.3 2.4-3.6 6-3.5 9.6.05 2.2 1 3.6 2.4 3.9 3.1.6 8.7-2.4 11.9-6.6.4-.55-1.3.9-2.4.6-1.8-.5-2.9-1.5-3.2-3.1-.3-2.3-2-3.7-4.2-3.3z"
        fill={fill}
      />
      <path d="M8.4 6.8c-.3-1.6.4-3 1.9-3.6M10.3 3.2c.7-.3 1.4-.25 2 .1" />
    </Ink>
  );
}
