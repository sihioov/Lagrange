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

## Add an optional widget

1. Add a widget component under `dashboard/widgets` or `detail/widgets`. Accept only
   `StockBetaWidgetProps<YourViewModel>` and keep the view model explicit.
2. Register it with `defineStockBetaWidget`. Supply a unique `id`, supported `defaultSize`, a
   non-negative unique `order`, `required: false`, and its default visibility.
3. Add a placement to each breakpoint where it should appear. A placement owns responsive
   `size`, `visible`, and `order`; omitting an optional widget from one breakpoint removes it there.
4. Assemble the registry, required-ID policy, and all three layouts with
   `defineStockBetaWidgetArchitecture`. Validation runs before rendering and rejects ambiguous or
   unsupported configuration.
5. Add an isolated render test for the widget and update the architecture test when layout policy
   changes.

The runtime registry includes React component functions and remains module-local. When layout
metadata must cross a Server/Client boundary or be persisted, derive it with
`stockBetaWidgetConfiguration()`. That projection contains only IDs, booleans, numbers, size
strings, and breakpoint placements and is JSON-serializable. Never pass the runtime registry as a
Client Component prop.

## Remove or reorder a widget

An optional widget can be removed from layout placements and then from the registry. Reordering is
only an `order` change in the applicable layouts; orders must remain unique and non-negative.

Required widgets are different. Their IDs must remain in the explicit `requiredWidgetIds` policy,
their definitions must have both `required: true` and `defaultVisible: true`, and every desktop,
tablet, and mobile layout must include them visibly. The validator fails closed if any of these
conditions diverge. Removing a required widget is a product-policy change, not a layout edit.

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
