import { StatePanel } from "@/components/states/state-panel";
import { WidgetFrame } from "../../shared/widget-frame";
import type { StockBetaDashboardWidgetViewModel } from "../types";

export function SignalStateWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, signalState } = viewModel;
  const state =
    signalState.kind === "unavailable"
      ? {
          kind: "blocked" as const,
          message: t.signalUnavailableMessage,
          title: t.signalUnavailableTitle,
        }
      : signalState.kind === "error"
        ? {
            kind: "error" as const,
            message: t.requestFailure(signalState.code),
            title: t.genericUnavailableTitle,
          }
        : { kind: "empty" as const, message: t.notReadyMessage, title: t.notReadyTitle };
  return (
    <WidgetFrame title={t.signalsHeading}>
      <StatePanel {...state} />
    </WidgetFrame>
  );
}
