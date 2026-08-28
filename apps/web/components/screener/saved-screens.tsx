"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import {
  deleteSavedScreenSchema,
  type SavedScreen,
  type ScreenCriteria,
  savedScreenSchema,
} from "@/lib/products/candidate-contracts";

type SaveState =
  | { readonly kind: "error" | "saved"; readonly message: string }
  | { readonly kind: "idle" | "submitting" };

export function SavedScreens({
  criteria,
  screens,
}: {
  readonly criteria: ScreenCriteria;
  readonly screens: readonly (SavedScreen & { readonly href: string })[];
}) {
  const router = useRouter();
  const [state, setState] = useState<SaveState>({ kind: "idle" });
  const [deleting, setDeleting] = useState<string | null>(null);

  async function save(form: HTMLFormElement): Promise<void> {
    const name = new FormData(form).get("name");
    if (typeof name !== "string" || name.trim() === "" || name.trim().length > 80) {
      setState({ kind: "error", message: "Enter a screen name of 1 to 80 characters." });
      return;
    }
    setState({ kind: "submitting" });
    try {
      const response = await mutateWithCsrf("/api/v1/screener/screens", {
        json: { criteria, name: name.trim() },
        method: "POST",
      });
      const saved = await parseBrowserApiResponse(response, savedScreenSchema);
      form.reset();
      setState({ kind: "saved", message: `Saved “${saved.name}”.` });
      router.refresh();
    } catch (error) {
      setState({
        kind: "error",
        message: error instanceof Error ? error.message : "The screen could not be saved.",
      });
    }
  }

  async function remove(id: string): Promise<void> {
    setDeleting(id);
    try {
      const response = await mutateWithCsrf(`/api/v1/screener/screens/${encodeURIComponent(id)}`, {
        json: {},
        method: "DELETE",
      });
      await parseBrowserApiResponse(response, deleteSavedScreenSchema);
      setState({ kind: "saved", message: "Saved screen deleted." });
      router.refresh();
    } catch (error) {
      setState({
        kind: "error",
        message: error instanceof Error ? error.message : "The screen could not be deleted.",
      });
    } finally {
      setDeleting(null);
    }
  }

  return (
    <section aria-labelledby="saved-screen-title" className="workflow-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Private workspace</p>
          <h2 id="saved-screen-title">Saved screens</h2>
        </div>
        <p>Saved criteria belong only to the signed-in user and are enforced by database RLS.</p>
      </div>
      <form
        aria-label="Save current screen"
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void save(event.currentTarget);
        }}
      >
        <label className="form-field">
          <span>Screen name</span>
          <input maxLength={80} name="name" required type="text" />
        </label>
        <button className="secondary-action" disabled={state.kind === "submitting"} type="submit">
          {state.kind === "submitting" ? "Saving screen" : "Save current criteria"}
        </button>
      </form>
      {state.kind === "error" || state.kind === "saved" ? (
        <p className="form-result" role={state.kind === "error" ? "alert" : "status"}>
          {state.message}
        </p>
      ) : null}
      {screens.length === 0 ? (
        <p className="empty-copy">No private screens have been saved.</p>
      ) : (
        <ul className="saved-screen-list">
          {screens.map((screen) => (
            <li key={screen.id}>
              <Link href={screen.href}>{screen.name}</Link>
              <button
                className="secondary-action"
                disabled={deleting === screen.id}
                onClick={() => void remove(screen.id)}
                type="button"
              >
                {deleting === screen.id ? "Deleting" : "Delete"}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
