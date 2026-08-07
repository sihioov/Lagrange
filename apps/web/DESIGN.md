# Lagrange Station Design System

## 0. Research Log

- Product brief: Todo 25 and the system requirements define an authenticated investment-research shell for invited Members and Owner/Admin operators; trust, isolation, and clear system state outrank spectacle.
- Embedded references: selected `taste-skill` + `layout-skill` for a regulated application shell and `revolut.md` for cool-neutral fintech discipline, pill actions, semantic color, and shadowless depth. Geist replaces proprietary Aeonik while preserving geometric clarity.
- UI/UX database: the query `fintech investment research dashboard trust-first cool neutral cobalt accessible content-dense` supported a dual-mode, WCAG-AA, data-dense dashboard and a professional sans family. Its newsletter layout and gold/purple palette were rejected because they conflict with the product model and the approved Revolut-inspired direction.
- Product documents: `Lagrange_Station_System_Design_v1.1.md` supplies the screen hierarchy; `Lagrange_Station_Requirements_v1.1.md` supplies Member/Owner permissions and conservative failure behavior.
- Skipped lanes: Lazyweb and Imagen were not used because the brief already supplies a concrete brand reference and an approved product-shell direction; this is not a clone or image-first concept exercise.
- Design dials: `DESIGN_VARIANCE 4`, `MOTION_INTENSITY 3`, `VISUAL_DENSITY 6`.

## 1. Atmosphere & Identity

Lagrange Station is a quiet risk console: precise, calm, and visibly conservative when data or permissions are uncertain. Cool neutral layers and a single cobalt signal line create the signature. The memorable moment is not decoration; it is the shell making role, freshness, and blocked states unmistakable before a user acts.

The interface serves two primary personas:

- **Member researcher**: configures approved strategies, reads recommendations, runs backtests, and manages a private paper account without seeing another user's data.
- **Owner operator**: performs the same research tasks and reaches explicitly separated administration and future live-control areas.

Ability-spectrum stress personas are keyboard-only users, low-vision users at 200% zoom, users who need reduced motion, and users under time pressure who need plain recovery instructions.

## 2. Color

### Palette

| Role | Token | Light | Dark | Usage |
|---|---|---:|---:|---|
| Canvas | `--surface-canvas` | `#F4F6F9` | `#0D1117` | App background |
| Panel | `--surface-panel` | `#FFFFFF` | `#141A22` | Main regions and cards |
| Muted surface | `--surface-muted` | `#EAEFF5` | `#1B2330` | Selected rows and quiet controls |
| Strong surface | `--surface-strong` | `#DDE4ED` | `#253041` | Pressed and emphasized regions |
| Primary text | `--text-primary` | `#171C24` | `#F3F6FA` | Headings and body |
| Secondary text | `--text-secondary` | `#536071` | `#AFBAC8` | Supporting copy |
| Tertiary text | `--text-tertiary` | `#5C6778` | `#8F9BAD` | Metadata only |
| Default border | `--border-default` | `#CDD6E2` | `#303C4C` | Region and control outlines |
| Subtle border | `--border-subtle` | `#DEE5EE` | `#25303E` | Dividers |
| Cobalt 400 | `--accent-soft` | `#E5E8FF` | `#252C63` | Selected background |
| Cobalt 600 | `--accent-primary` | `#4351D8` | `#8790FF` | Links, active nav, primary action |
| Cobalt 700 | `--accent-hover` | `#3441BB` | `#A1A8FF` | Hover and focus emphasis |
| On accent | `--accent-on` | `#FFFFFF` | `#10142A` | Text on accent |
| Success | `--status-success` | `#087A57` | `#56D5AA` | Positive status with text/icon |
| Warning | `--status-warning` | `#9B5700` | `#FFB76B` | Caution with text/icon |
| Error | `--status-error` | `#B83344` | `#FF7C89` | Failure/destructive with text/icon |
| Information | `--status-info` | `#245EA8` | `#79AAFF` | Informational state with text/icon |
| Focus | `--focus-ring` | `#4351D8` | `#A1A8FF` | Two-pixel keyboard focus ring |

### Rules

- Cobalt is interactive, never decorative. Semantic colors always include an icon or explicit label.
- Surfaces stay in one cool-neutral family. No warm gray, neon, purple glow, or gradient text.
- Light and dark values are paired at the token layer using `prefers-color-scheme`; components never choose theme-specific raw colors.
- Body contrast targets WCAG 2.2 AA at minimum; primary reading text targets AAA where practical.

## 3. Typography

### Font Stack

- Primary: Geist Sans from the installed `geist` package, then `system-ui`, sans-serif.
- Data: Geist Mono, then `ui-monospace`, monospace.
- No serif family. Numeric results use tabular figures.

### Scale

| Level | Size | Weight | Line height | Tracking | Usage |
|---|---:|---:|---:|---:|---|
| Display | `2.5rem` | 560 | 1.08 | `-0.035em` | Login statement only |
| H1 | `2rem` | 560 | 1.18 | `-0.025em` | Page title |
| H2 | `1.5rem` | 560 | 1.25 | `-0.015em` | Major region |
| H3 | `1.125rem` | 560 | 1.35 | `-0.01em` | Panel title |
| Body | `1rem` | 420 | 1.55 | `0` | Default reading text |
| Body small | `0.875rem` | 430 | 1.5 | `0.005em` | Secondary information |
| Label | `0.75rem` | 560 | 1.35 | `0.04em` | Short metadata labels |
| Data | `0.875rem` | 500 | 1.45 | `0` | IDs, dates, numbers |

Body text never drops below `0.875rem`. Labels remain short and never carry essential instructions alone.

## 4. Spacing & Layout

### Tokens

All spacing intent derives from a four-pixel unit.

| Token | Value | Usage |
|---|---:|---|
| `--space-1` | `0.25rem` | Icon micro-gap |
| `--space-2` | `0.5rem` | Tight cluster |
| `--space-3` | `0.75rem` | Compact control inset |
| `--space-4` | `1rem` | Standard inset |
| `--space-5` | `1.25rem` | Comfortable inset |
| `--space-6` | `1.5rem` | Panel padding |
| `--space-8` | `2rem` | Page-region gap |
| `--space-10` | `2.5rem` | Major separation |
| `--space-12` | `3rem` | Wide-page gutter |

### Shell Contract

- Maximum content width: `90rem`; readable prose remains within `65ch`.
- At `48rem` and wider, the shell is a fixed-sidenav grid. The shell is bounded by `100dvb`; `<main>` is the only vertical scroll owner and has `min-block-size: 0`.
- Below `48rem`, document scroll owns the page. Header, role context, and a wrapping top-level navigation precede main content; primary content never requires horizontal scrolling.
- Intrinsic content grids use `repeat(auto-fit, minmax(min(16rem, 100%), 1fr))`.
- Breakpoint evidence is required at `375px`, `768px`, and `1280px`, plus 200% zoom.
- Long labels wrap deliberately; unbroken identifiers use `overflow-wrap: anywhere` and never widen the shell.

## 5. Components

### Application Shell

- **Structure**: skip link → header/identity → `nav[aria-label="Primary"]` → `main#main-content` → spatially separated sign-out form.
- **Variants**: Member navigation and Owner navigation. Owner-only destinations are absent for Members, not merely disabled.
- **States**: current destination uses `aria-current="page"`; hover, active, and focus remain visible in both themes.
- **Accessibility**: landmark names are stable; visual order equals DOM order; every target is at least `44px` high.
- **Layout**: `fixed-sidenav-shell` on wide screens, document-scroll stack on narrow screens.

### Primary Navigation Item

- **Structure**: Phosphor regular-weight icon plus visible text label inside a real link.
- **Variants**: default, current, Owner-only.
- **States**: hover uses muted surface; active uses strong surface; current adds the cobalt signal line and text weight; focus uses the focus token.
- **Motion**: color and transform feedback only; no automatic movement.

### Action

- **Structure**: native button or link with visible label; icons supplement text.
- **Variants**: primary cobalt pill, secondary neutral pill, quiet text action, destructive semantic action.
- **States**: hover, pressed, focus-visible, disabled, and busy. Busy actions retain their label and expose status text.
- **Accessibility**: minimum `44px` target; primary labels stay on one line; disabled semantics are native.

### Surface Panel

- **Structure**: optional heading and metadata followed by content; no card wrapper when spacing alone communicates grouping.
- **Variants**: panel, muted panel, outlined data region.
- **States**: static by default; interactive panels become links or buttons rather than clickable generic containers.
- **Depth**: tonal shift plus one subtle hairline. No drop shadow.

### State Panel

- **Structure**: Phosphor status icon, title, plain-language message, optional action.
- **Variants**: loading, empty, error, blocked entitlement.
- **States**: loading and empty use `role="status"` with polite announcements; error and blocked use `role="alert"`. Loading skeletons match final geometry and do not shimmer under reduced motion.
- **Recovery**: error text states what failed and what the action does; blocked entitlement never hints that retrying will bypass policy.

### Page Frame

- **Structure**: page header, optional compact metadata cluster, then content regions.
- **Layout**: `stack` within the shell's content limiter. Each page has exactly one `h1`.
- **Responsive**: actions wrap below the title before labels truncate.

### Status Pill

- **Structure**: status icon or dot plus explicit status text.
- **Variants**: neutral, information, success, warning, error.
- **Accessibility**: color is secondary; text always carries the meaning.

### Schema-bound Configuration Form

- **Structure**: named native form, server-supplied parameter fieldset, short parameter help, submit action, and one live result region.
- **Variants**: numeric bounds, enumerated select, text, and boolean controls derived from the approved schema; arbitrary code and untyped free-form payloads are absent.
- **States**: ready, submitting, saved, invalid, and blocked. Client checks improve recovery, while the API remains authoritative and returns stable error codes.
- **Accessibility**: every field has a visible label; invalid state uses `role="alert"`; blocked controls are not rendered as an apparent bypass.

### Data Report

- **Structure**: named report region, status/as-of cluster, warning strip, responsive data regions, and a provenance footer.
- **Variants**: recommendation, backtest result, comparison, and robustness evidence. Every variant displays strategy, data, and engine versions plus a warnings section.
- **States**: ready, stale, empty subsection, failed, canceled, integrity error, and entitlement blocked. Proprietary rows are absent from blocked markup.
- **Language**: recommendation output is labeled a strategy-based proposal and warning; it never uses guaranteed-return or personalized investment language.

### Data Table

- **Structure**: caption, semantic column headers, compact rows, and an overflow-safe wrapper for genuinely tabular secondary content.
- **Variants**: candidates, exclusions, run history, metrics, monthly returns, trades, costs, provenance, comparisons, and robustness evidence.
- **Accessibility**: captions name the dataset, row/column headers remain explicit, and empty data is prose rather than a blank table.

### Report Provenance Footer

- **Structure**: labeled strategy, data, and engine versions; report as-of time; license state; and warning evidence.
- **States**: complete metadata or explicit `Not reported` values with a warning. Missing metadata is never silently hidden.

## 6. Motion & Interaction

| Token | Duration | Easing | Usage |
|---|---:|---|---|
| `--motion-press` | `100ms` | `cubic-bezier(0.2, 0, 0, 1)` | Button press |
| `--motion-micro` | `140ms` | `cubic-bezier(0.16, 1, 0.3, 1)` | Hover and focus response |
| `--motion-state` | `200ms` | `cubic-bezier(0.16, 1, 0.3, 1)` | Local state replacement |

- Motion intensity is 3: no automatic page entrances, parallax, marquees, magnetic effects, or decorative loops.
- Interactive feedback may animate only `transform`, `opacity`, and color. Press feedback uses a subtle `scale(0.98)` without moving surrounding layout.
- `prefers-reduced-motion: reduce` removes interaction transforms and makes state changes immediate; the skip link remains off-canvas until focus without animating.
- Focus is never delayed by animation. Loading feedback appears without blocking navigation.

## 7. Depth & Surface

Depth strategy is **tonal shift with selective hairlines**. This adapts Revolut's shadowless confidence to a data-heavy shell.

- Canvas, panel, muted, and strong tokens create four visible planes.
- Cards use a `1rem` radius; compact controls use `0.75rem`; action buttons use full pills. This radius hierarchy is fixed.
- No `box-shadow`, ambient glow, backdrop blur, glass panel, or gradient card is part of the shell.
- The cobalt signal line is reserved for current navigation and focused workflow context.

## 8. Accessibility Constraints & Accepted Debt

### Constraints

- Target WCAG 2.2 AA: body contrast at least 4.5:1, large text and UI graphics at least 3:1.
- Provide a skip link, named landmarks, one `h1`, visible `:focus-visible`, complete keyboard reachability, and logical source order.
- Touch targets are at least `44px`; no primary interaction relies on hover, color, drag, or precise pointer movement.
- Preserve browser zoom and reflow at 200%. At `375px`, primary content is one readable column with no horizontal page scroll.
- Error and blocked states are announced and include truthful recovery guidance. Loading uses polite live regions and avoids layout shift.
- Honor `prefers-color-scheme`, `prefers-reduced-motion`, forced colors, and increased text size. Icons are decorative when adjacent text already names the action.
- Authentication and authorization state is never inferred from hidden navigation; server policy remains authoritative.

### Accepted Debt

None. Any later visual or accessibility debt must name the affected users, exact location, severity, repair, owner, and exit condition before acceptance.
