"use client";

import { useState } from "react";
import { logout } from "@/lib/api/browser-client";

type LogoutState = "idle" | "submitting" | "error";

export function LogoutForm() {
  const [state, setState] = useState<LogoutState>("idle");

  async function submitLogout(): Promise<void> {
    setState("submitting");
    try {
      const response = await logout();
      if (response.ok) {
        window.location.assign("/login");
        return;
      }
      setState("error");
    } catch (error) {
      if (error instanceof Error) {
        setState("error");
        return;
      }
      throw error;
    }
  }

  return (
    <form
      aria-label="Sign out"
      className="shell-signout"
      onSubmit={(event) => {
        event.preventDefault();
        void submitLogout();
      }}
    >
      <button disabled={state === "submitting"} type="submit">
        {state === "submitting" ? "Signing out" : "Sign out"}
      </button>
      {state === "error" ? (
        <p role="alert">Sign out failed. Check your connection and retry.</p>
      ) : null}
    </form>
  );
}
