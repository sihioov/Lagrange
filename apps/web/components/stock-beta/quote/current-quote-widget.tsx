import type { StockBetaDashboardWidgetViewModel } from "../dashboard/types";
import type { StockBetaDetailWidgetViewModel } from "../detail/types";
import type { StockBetaWidgetProps } from "../shared/widget-types";
import { CurrentQuoteClient } from "./current-quote-client";
import {
  intradayQuoteIdentityForMembership,
  matchReadyIntradayQuoteMembership,
} from "./membership";

type CurrentQuoteWidgetViewModel =
  | StockBetaDashboardWidgetViewModel
  | StockBetaDetailWidgetViewModel;

function isDashboardViewModel(
  viewModel: CurrentQuoteWidgetViewModel,
): viewModel is StockBetaDashboardWidgetViewModel {
  return "signals" in viewModel;
}

export function CurrentQuoteWidget({
  placementVisibility,
  viewModel,
}: StockBetaWidgetProps<CurrentQuoteWidgetViewModel>) {
  const dashboard = isDashboardViewModel(viewModel);
  const signal = dashboard
    ? (viewModel.signals?.rows.find(
        (row) => row.instrument_id === viewModel.selectedInstrumentId,
      ) ??
      viewModel.signals?.rows[0] ??
      null)
    : viewModel.detail.signal;
  const selectedMembership = dashboard
    ? matchReadyIntradayQuoteMembership(viewModel.memberships, {
        instrument_id: signal?.instrument_id ?? "000000.KRX",
        generation: signal?.generation ?? 0,
      })
    : (viewModel.intradayMembership ?? null);
  const identity =
    signal === null || selectedMembership === null
      ? null
      : intradayQuoteIdentityForMembership(selectedMembership);
  const snapshotKey =
    signal === null
      ? null
      : [
          dashboard
            ? viewModel.signals?.snapshot.snapshot_id
            : viewModel.detail.snapshot.snapshot_id,
          dashboard ? viewModel.signals?.snapshot.as_of : viewModel.detail.snapshot.as_of,
          signal.instrument_id,
          String(signal.generation),
        ].join("\u0000");
  const t = viewModel.copy;
  return (
    <CurrentQuoteClient
      copy={{
        intradayQuoteBasePriceLabel: t.intradayQuoteBasePriceLabel,
        intradayQuoteBasePricePolicy: t.intradayQuoteBasePricePolicy,
        intradayQuoteChangeLabel: t.intradayQuoteChangeLabel,
        intradayQuoteChangePercentLabel: t.intradayQuoteChangePercentLabel,
        intradayQuoteClosed: t.intradayQuoteClosed,
        intradayQuoteDemanding: t.intradayQuoteDemanding,
        intradayQuoteDescription: t.intradayQuoteDescription,
        intradayQuoteDirectionDown: t.intradayQuoteDirectionDown,
        intradayQuoteDirectionFlat: t.intradayQuoteDirectionFlat,
        intradayQuoteDirectionLabel: t.intradayQuoteDirectionLabel,
        intradayQuoteDirectionLimitDown: t.intradayQuoteDirectionLimitDown,
        intradayQuoteDirectionLimitUp: t.intradayQuoteDirectionLimitUp,
        intradayQuoteDirectionUp: t.intradayQuoteDirectionUp,
        intradayQuoteHalted: t.intradayQuoteHalted,
        intradayQuoteHeading: t.intradayQuoteHeading,
        intradayQuoteLastSuccessLabel: t.intradayQuoteLastSuccessLabel,
        intradayQuoteNoActiveDemand: t.intradayQuoteNoActiveDemand,
        intradayQuoteOffline: t.intradayQuoteOffline,
        intradayQuotePolling: t.intradayQuotePolling,
        intradayQuotePriceLabel: t.intradayQuotePriceLabel,
        intradayQuoteRefreshFailed: t.intradayQuoteRefreshFailed,
        intradayQuoteReady: t.intradayQuoteReady,
        intradayQuoteStale: t.intradayQuoteStale,
        intradayQuoteUnknown: t.intradayQuoteUnknown,
        intradayQuoteUnavailable: t.intradayQuoteUnavailable,
      }}
      enabled={viewModel.intradayEnabled === true}
      identity={identity}
      locale={viewModel.locale}
      {...(placementVisibility === undefined ? {} : { placementVisibility })}
      sessionKey={snapshotKey}
      surface={dashboard ? "dashboard" : "detail"}
    />
  );
}
