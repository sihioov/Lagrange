import { StatePanel } from "@/components/states/state-panel";

export default function AuthenticatedLoading() {
  return (
    <StatePanel
      kind="loading"
      message="The authenticated workspace is requesting current data without using a shared cache."
      title="Loading workspace"
    />
  );
}
