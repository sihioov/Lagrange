# Lagrange Station Design System

## 0. Research Log

- Product brief: Todo 25 and the system requirements define an authenticated investment-research shell for invited Members and Owner/Admin operators; trust, isolation, and clear system state outrank spectacle.
- Subject grounding: the product's own name supplied the visual direction that a generic fintech reference could not. A Lagrange point is where a small body holds a stable position between the gravity of two much larger ones — the exact shape of what this console does for four or five people navigating a market with real money on one side and their own discipline on the other. The redesign is built from that subject's own vernacular: orbital mechanics, mission-telemetry readouts, and precision instrument dials, not a fintech-dashboard mood board.
- Rejected defaults: this pass was checked against the three visual patterns that recur regardless of brief — warm cream canvas with a high-contrast serif and a terracotta accent; near-black canvas with one neon accent; and zero-radius newspaper hairlines. The canvas is cool-neutral rather than cream, the accent is a desaturated instrument-steel blue rather than a saturated neon, and the pill/rounded-panel radius system from the prior pass is kept rather than flattened to a broadsheet grid. None of the three defaults apply here.
- Superseded reference: the initial pass borrowed `revolut.md`'s cool-neutral cobalt fintech discipline directly. That palette read as competent but generic — the same shell a hundred other invite-only fintech tools could wear. This revision keeps the shadowless, pill-and-hairline depth strategy (still correct for a regulated, data-dense shell) but replaces the borrowed cobalt identity with one derived from the product's own name.
- Product documents: `Lagrange_Station_System_Design_v1.1.md` supplies the screen hierarchy; `Lagrange_Station_Requirements_v1.1.md` supplies Member/Owner permissions and conservative failure behavior.
- Design dials: `DESIGN_VARIANCE 6`, `MOTION_INTENSITY 3`, `VISUAL_DENSITY 6`. Variance moved up from the first pass because the palette and type system are no longer a borrowed reference; motion and density are unchanged — this pass restyles the shell, it does not add animation or change how much information a screen carries.

## 1. Atmosphere & Identity

Lagrange Station is a quiet risk console: precise, calm, and visibly conservative when data or permissions are uncertain. A cool graphite-and-vellum surface family, a single instrument-steel accent, and one reserved signature color read as a mission console, not a consumer fintech app. The memorable moment is not decoration; it is the shell making role, freshness, and blocked states unmistakable before a user acts.

### 1.1 Signature: the equilibrium mark

The one deliberate risk this pass takes is a small custom glyph — two solid points (the large and small body) triangulated against a third, lighter point where the two forces balance. It is drawn once (`components/shell/equilibrium-mark.tsx`) and used in exactly two places, never more: the header wordmark, and the icon inside every `State Panel`, tinted to the panel's status color. The same figure that means "in balance" in the header is legible as "out of balance" when it turns warning-amber or error-red — the mark is not a logo bolted onto the shell, it is load-bearing UI. The third, marked point uses `--signature` and appears nowhere else in the interface; every other interactive surface uses `--accent-primary`, so a reader never has to wonder whether the gold point is clickable.

The interface serves two primary personas:

- **Member researcher**: configures approved strategies and manages owned runs/accounts while reading backtest reports and Paper account activity shared across the invite group.
- **Owner operator**: performs the same research tasks and reaches explicitly separated administration and future live-control areas.

Ability-spectrum stress personas are keyboard-only users, low-vision users at 200% zoom, users who need reduced motion, and users under time pressure who need plain recovery instructions.

## 2. Color

### Palette

| Role | Token | Light | Dark | Usage |
|---|---|---:|---:|---|
| Vellum (canvas) | `--surface-canvas` | `#EBEFF0` | `#0D1116` | App background |
| Panel | `--surface-panel` | `#FBFCFC` | `#131A20` | Main regions and cards |
| Muted surface | `--surface-muted` | `#E1E7E8` | `#1A232A` | Selected rows and quiet controls |
| Strong surface | `--surface-strong` | `#D1D9DA` | `#29343E` | Pressed and emphasized regions |
| Deep Ink (primary text) | `--text-primary` | `#0E1417` | `#F3F7F8` | Headings and body |
| Secondary text | `--text-secondary` | `#3E4D51` | `#B7C4C7` | Supporting copy |
| Tertiary text | `--text-tertiary` | `#57696D` | `#8BA0A3` | Metadata only |
| Default border | `--border-default` | `#B7C2C4` | `#33414B` | Region and control outlines |
| Subtle border | `--border-subtle` | `#D0D9DB` | `#232F38` | Dividers |
| Accent soft | `--accent-soft` | `#D2EEF5` | `#0F3745` | Selected background |
| Steel Blue (accent) | `--accent-primary` | `#077299` | `#4FC6E8` | Links, active nav, primary action |
| Steel Blue, hover | `--accent-hover` | `#045A7A` | `#7ADCF2` | Hover and focus emphasis |
| On accent | `--accent-on` | `#FFFFFF` | `#04141A` | Text on accent |
| Signature Brass | `--signature` | `#8C6310` | `#E7B65E` | The equilibrium mark's third point, and the eyebrow micro-label color — never a button, link, or other clickable surface |
| Success | `--status-success` | `#0E8C5C` | `#4FE0A8` | Positive status with text/icon |
| Warning | `--status-warning` | `#B4650F` | `#F0A855` | Caution with text/icon |
| Error | `--status-error` | `#B8264A` | `#FF8592` | Failure/destructive with text/icon |
| Information | `--status-info` | `#2E6FB0` | `#7EC0F2` | Informational state with text/icon |
| Focus | `--focus-ring` | `#077299` | `#4FC6E8` | Two-pixel keyboard focus ring |

### Rules

- Steel Blue is interactive, never decorative. Semantic colors always include an icon or explicit label.
- Signature Brass marks exactly one shape (the equilibrium mark's third point) plus the recurring eyebrow micro-label, and is never applied to a button, link, or any other clickable surface — the rule that keeps "gold" from being mistaken for "clickable."
- Surfaces stay in one cool-neutral family. The canvas is a graphite/vellum grey, deliberately not a warm cream — a high-contrast serif-on-cream pairing is the single most common AI-generated default and is rejected outright here. No warm gray, neon, purple glow, or gradient text.
- Light and dark values are paired at the token layer; components never choose theme-specific raw colors. Theme resolution has two layers: `prefers-color-scheme` picks a default, and an explicit `data-theme="dark"`/`data-theme="light"` attribute on `<html>` — set by the header's theme toggle and persisted in a `theme` cookie — overrides it in either direction. A first-time visitor with no cookie yet gets the OS default with zero extra requests; a returning visitor's explicit choice is read server-side before the first byte, so there is no flash of the wrong theme.
- Body contrast targets WCAG 2.2 AA at minimum; primary reading text targets AAA where practical. Every accent/status pairing above is re-verified by `tests/shell-runtime.test.ts`'s automated contrast check, not just eyeballed — a future palette change that regresses contrast fails CI, not just review.
- Round 2 (this pass) increased accent saturation and contrast in response to feedback that the round-1 palette read as muted; the hue families (cool graphite neutrals, blue accent, warm brass signature) are unchanged from round 1, only the specific values.

## 3. Typography

### Font Stack

- Body: Geist Sans from the installed `geist` package, then `system-ui`, sans-serif. Unchanged from the first pass — it is already legible, AA-safe, and geometrically neutral; nothing about the redesign required replacing it.
- Display (`--font-display`): Geist Mono, then `ui-monospace`, monospace — reused, not added, and given an unusual job. Page `h1`s, the uppercase eyebrow micro-labels, and data-table column headers are set in mono rather than the humanist sans, so short strings read like a mission-console readout ("BACKTESTS", "QUEUED EXECUTION") instead of a marketing headline. Reserved for short strings only — every place it is applied in this shell holds one to three words.
- Data: Geist Mono, then `ui-monospace`, monospace. IDs, dates, and tabular numbers — unchanged from the first pass.
- No serif family. Numeric results use tabular figures.

### Scale

| Level | Size | Weight | Line height | Tracking | Font | Usage |
|---|---:|---:|---:|---:|---|---|
| Display | `2.5rem` | 560 | 1.08 | `-0.035em` | Body | Login statement only |
| H1 | `clamp(1.375rem, 3.4vw, 1.875rem)` | 600 | 1.22 | `-0.01em` | Display (mono) | Page title |
| H2 | `1.5rem` | 560 | 1.25 | `-0.015em` | Body | Major region |
| H3 | `1.125rem` | 560 | 1.35 | `-0.01em` | Body | Panel title |
| Body | `1rem` | 420 | 1.55 | `0` | Body | Default reading text |
| Body small | `0.875rem` | 430 | 1.5 | `0.005em` | Body | Secondary information |
| Label / eyebrow | `0.6875rem` | 600 | 1.35 | `0.08em` | Display (mono) | Uppercase micro-labels |
| Table header | `0.6875rem` | 500 | 1.35 | `0.06em` | Display (mono) | Column headers |
| Data | `0.875rem` | 500 | 1.45 | `0` | Data (mono) | IDs, dates, numbers |

Body text never drops below `0.875rem`. Labels remain short and never carry essential instructions alone. H2 and H3 stay in Geist Sans rather than the mono display face — a dashboard region can carry several of them, and mono headings repeated at that density would read as a gimmick rather than a readout. The mono display treatment is spent once per page (the `h1`) and on labels genuinely short enough to look intentional in a fixed-width face.

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

- **Structure**: Phosphor regular-weight icon plus visible text label inside a real link. The icon is `aria-hidden` — it is a scanning aid, not part of the accessible name — and chosen to name the destination's instrument rather than a generic dashboard glyph: `Planet` (Dashboard, the body the station orbits), `Target` (Strategies, precision), `Compass` (Recommendations, heading), `Flask` (Backtests, experiment), `Wallet` (Paper account), `Gauge` (Administration), `Broadcast` (Live controls, a signal leaving the station).
- **Variants**: default, current, Owner-only.
- **States**: hover uses muted surface; active uses strong surface; current adds the Steel Blue signal line, tints its icon to match, and increases text weight; focus uses the focus token.
- **Motion**: color and transform feedback only; no automatic movement.

### Action

- **Structure**: native button or link with visible label; icons supplement text.
- **Variants**: primary Steel Blue pill (`.primary-action`), secondary neutral pill (`.secondary-action`), quiet text action (`.quiet-action`, no fill or border — used for a low-stakes navigational action next to or instead of a real primary, e.g. "Return to dashboard" on an Owner-only refusal), caution pill (`.caution-action`, filled with `--status-warning` — used for the more consequential direction of a two-way safety control, e.g. disengaging the Live kill switch, never for the safe direction of the same control).
- **States**: hover, pressed, focus-visible, disabled, and busy. Busy actions retain their label and expose status text.
- **Accessibility**: minimum `44px` target; primary labels stay on one line; disabled semantics are native.

### Surface Panel

- **Structure**: optional heading and metadata followed by content; no card wrapper when spacing alone communicates grouping.
- **Variants**: panel, muted panel, outlined data region.
- **States**: static by default; interactive panels become links or buttons rather than clickable generic containers.
- **Depth**: tonal shift plus one subtle hairline. No drop shadow.

### State Panel

- **Structure**: the equilibrium mark (see §1.1), title, plain-language message, optional action. The mark's color follows the panel's `data-kind`, so the one glyph the shell uses to mean "in balance" is also how it shows "out of balance."
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

Depth strategy is **tonal shift with selective hairlines** — shadowless confidence carried over unchanged from the first pass, because it was already correct for a data-heavy shell and owed nothing to the borrowed cobalt identity it is now paired with instead.

- Canvas, panel, muted, and strong tokens create four visible planes.
- Cards use a `1rem` radius; compact controls use `0.75rem`; action buttons use full pills. This radius hierarchy is fixed.
- No `box-shadow`, ambient glow, backdrop blur, glass panel, or gradient card is part of the shell.
- The Steel Blue signal line is reserved for current navigation and focused workflow context. Signature Brass never appears here — it marks the equilibrium point, not interactive state.

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
