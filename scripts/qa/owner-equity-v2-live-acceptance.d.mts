export type ExtractedQuoteDom = {
  instrument_id: string;
  last_success_at: string;
  price: string;
  status_phase: "ready";
};

export function extractQuoteDom(
  expectedInstrument: string,
  rootDocument?: Document | null,
): ExtractedQuoteDom | null;
