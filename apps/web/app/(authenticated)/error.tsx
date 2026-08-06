"use client";

import { StatePanel } from "@/components/states/state-panel";

export type AuthenticatedErrorProps = {
  readonly reset: () => void;
};

export default function AuthenticatedError({ reset }: AuthenticatedErrorProps) {
  return (
    <StatePanel
      action={
        <button className="primary-action" onClick={reset} type="button">
          Try again
        </button>
      }
      kind="error"
      message="The authenticated request could not be completed. Retry the request without reusing a cached response."
      title="We could not load this workspace"
    />
  );
}
