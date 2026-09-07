# Stock-beta widget architecture

## Authenticated terminal shell boundary

`AppShell` passes one live page slot and the authenticated shell inputs to `RouteAwareShell`, which
mounts the shared `components/shell/ResearchTerminalShell` for every authenticated route.
`usePathname()` selects the current product label and the Stock Beta-specific utility state; it does
not swap the application back to a legacy shell. There is no hidden second shell, `:has()` recolor,
or duplicate landmark.

The shell owns the dark terminal tokens, 50px utility bar, primary navigation rail, compact panel
language, responsive scroll ownership, locale control, role context, and sign-out action. Product
pages keep their semantic components and API behavior while inheriting the same shell and density.
Adding another authenticated destination therefore requires one navigation entry and one route;
it must not introduce a product-local application shell.

DTO-dependent chrome belongs in `StockBetaTerminalPage`. Its typed `search`, `asOf`, `snapshot`,
and `titleTools` slots let the server page compose current response data without making the product
shell fetch or invent placeholder controls. Omit a slot when the page has no working control or
truthful value. The terminal shell deliberately has no theme, market, alert, export, add-widget, or
layout button.

`search` and `asOf` use `StockBetaTerminalUtilitySlot`. The shared shell owns one
`StockBetaTerminalUtilityHost` inside its 50px header, and the slot portals the existing page nodes
into that host after hydration. React portals preserve the dashboard selection context, so search
stays under `StockBetaSelectionProvider`; no node is cloned. The server render and first hydration
render both omit portal content until the host ref commits, preventing a hydration mismatch. Error
and refusal pages can leave the route-scoped host empty.

The shared foundation keeps fetching and DTO validation outside widgets. A page or controller must
fetch once, parse with the existing strict equity-signal contract, and pass a typed view model to
each registered widget. Widgets render those values; they do not call `fetch`, product clients, or
route handlers.

## Extend the selected profile tabs

`dashboard/profile-tab-registry.ts` is the sole ordered registry for the selected profile tabs.
Add, remove, or reorder an entry there; `SignalPreviewWidget` renders the registry without a
tab-specific control-flow branch. A tab renderer receives the selected V2 row and the already-loaded
dashboard view model only. The Price renderer may use `chartData`, `chartState`, `chartRange`, and
`onChartRangeChange`, plus the presentation-only `PriceChart`; it must not import a fetcher, URL
builder, product/API client, Zod schema, or `AbortController`.

Price data is the latest completed EOD close, not a real-time quote. It is explicitly original /
unadjusted and must always retain the corporate-action caveat. Integrity and unavailable/error
states fail closed: do not leave a previous chart or inferred price visible.

## Add an optional widget

1. Add a widget component under `dashboard/widgets` or `detail/widgets`. Accept only
   `StockBetaWidgetProps<YourViewModel>` and keep the view model explicit.
2. Add one entry to the screen's catalog with its unique `id`, component, `required: false`, and
   `placements`. Each breakpoint placement owns its responsive size and visibility; dashboard
   placements also own populated and empty grid coordinates. Omitting an optional breakpoint
   placement removes the widget there.
3. Keep the entry at its intended reading position. Catalog array order is the canonical DOM and
   accessibility order; grid coordinates express visual placement only. The existing CSS order
   custom properties are compatibility outputs derived from catalog index, never authoring
   metadata.
4. Let `defineStockBetaWidgetArchitecture` derive the required IDs and breakpoint layout. The
   catalog validator rejects duplicate IDs, invalid or overlapping grids, and incomplete required
   placements before rendering.
5. Add an isolated render test for the widget and update the architecture test when placement
   policy changes.

The runtime registry includes React component functions and remains module-local. When layout
metadata must cross a Server/Client boundary or be persisted, derive it with
`stockBetaWidgetConfiguration()`. That deterministic projection contains component-free widget
policy, derived required IDs, and breakpoint placements made only from IDs, booleans, numbers, and
size strings. Never pass the runtime catalog as a Client Component prop.

## Remove or reorder a widget

An optional widget is removed by deleting its catalog entry. Reorder widgets by moving entries in
the catalog array; the renderer follows that order in the DOM and derives compatibility CSS order
values from the same array index. No layout or ID list is synchronized separately.

Required widgets are different. Set `required: true` on the catalog entry and provide a visible
populated placement at desktop, tablet, and mobile. Dashboard required entries must also provide a
complete empty-state placement at every breakpoint; its empty visibility may remain false when the
accepted fail-closed UI hides that widget. `requiredWidgetIds` is derived from these entries.
Removing a required widget is a product-policy change, not a layout edit.

## Numeric values and states

`formatStockBetaNumber` and `formatStockBetaPercent` reject non-finite input and return both the
unchanged DTO `rawValue` and localized display `text`. Comparisons, ordering, chart geometry, and
conditions must use `rawValue`; formatted text is display-only. Do not normalize, clamp, rerank,
coerce, or synthesize a DTO value in a widget.

Use `WidgetFrame` for the named region, heading, status, and ready/loading/empty/error/blocked
state. Non-ready states intentionally do not render `children`, preventing stale or unverified data
from leaking through an error or integrity boundary. The dashboard shell, policy, snapshot,
condition summary, filters, and provenance widgets are Server Components. Row selection is isolated
in `dashboard/selection-provider.tsx`; only ranked-signals and selected-preview are Client
Components and consume that context. The provider receives server-rendered children, so
registry/layout placement stays centralized while static widgets remain outside the client module
graph.
