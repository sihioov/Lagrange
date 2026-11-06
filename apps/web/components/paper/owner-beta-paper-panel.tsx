"use client";

import { useRouter } from "next/navigation";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { ApiProblem } from "@/lib/api/response";
import { useLocale } from "@/lib/i18n/client";
import { paperDictionary } from "@/lib/i18n/dictionaries/paper";
import {
  OWNER_BETA_PAPER_CONSENT_PATH,
  OWNER_BETA_PAPER_PREVIEWS_PATH,
  OWNER_BETA_PAPER_STATE_PATH,
  type OwnerBetaPaperApplyModel,
  type OwnerBetaPaperPreviewModel,
  type OwnerBetaPaperSessionDetailModel,
  type OwnerBetaPaperStateModel,
  ownerBetaPaperApplyBodySchema,
  ownerBetaPaperApplyPath,
  ownerBetaPaperApplySchema,
  ownerBetaPaperConsentBodySchema,
  ownerBetaPaperConsentSchema,
  ownerBetaPaperPreviewBodySchema,
  ownerBetaPaperPreviewPath,
  ownerBetaPaperPreviewSchema,
  ownerBetaPaperSessionDetailSchema,
  ownerBetaPaperSessionPath,
  ownerBetaPaperStateSchema,
  type PaperOrderModel,
} from "@/lib/products/paper-contracts";

export type OwnerBetaPaperRunOption = {
  readonly asOf: string;
  readonly id: string;
  readonly strategyConfigId: string;
  readonly strategyLabel: string;
};

export type OwnerBetaPaperPanelProps = {
  readonly actualOrders?: readonly PaperOrderModel[];
  readonly initialError?: boolean;
  readonly initialState: OwnerBetaPaperStateModel | null;
  readonly runs: readonly OwnerBetaPaperRunOption[];
};

type PanelError = { readonly code: string; readonly kind: "error" | "unknown-commit" };

const POLL_DELAYS_MS = [750, 1_500, 3_000, 5_000, 8_000] as const;
const MAX_PREVIEW_POLLS = 10;
const UNKNOWN_COMMIT_ACCOUNT_KEY = "unknown-account";
const HINT_FRAGMENT_PREFIX = "#paper:";
const HINT_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const NIL_UUID = "00000000-0000-0000-0000-000000000000";

function previewHintKey(accountId: string): string {
  return `preview:${accountId}`;
}

function previewTargetHintKey(accountId: string): string {
  return `target:${accountId}`;
}

function unknownCommitHintKey(accountId: string): string {
  return `unknown:${accountId}`;
}

function validHintUuid(value: string): boolean {
  return HINT_UUID.test(value) && value !== NIL_UUID;
}

function validHintKey(key: string): boolean {
  if (key === UNKNOWN_COMMIT_ACCOUNT_KEY) return true;
  const separator = key.indexOf(":");
  return (
    ["preview", "target", "unknown"].includes(key.slice(0, separator)) &&
    validHintUuid(key.slice(separator + 1))
  );
}

function readHintParams(): URLSearchParams {
  const fragment = window.location.hash;
  if (!fragment.startsWith(HINT_FRAGMENT_PREFIX) || fragment.length > 4096) {
    return new URLSearchParams();
  }
  const params = new URLSearchParams(fragment.slice(HINT_FRAGMENT_PREFIX.length));
  const seen = new Set<string>();
  for (const [key, value] of params) {
    if (seen.has(key) || !validHintKey(key) || !validHintUuid(value)) {
      return new URLSearchParams();
    }
    seen.add(key);
  }
  return params;
}

function replaceHintParams(params: URLSearchParams): void {
  // Only non-secret UUID pointers enter the fragment. All account, token and epoch
  // authority is read again from the authenticated API before an action.
  const url = new URL(window.location.href);
  url.hash = params.size === 0 ? "" : `${HINT_FRAGMENT_PREFIX.slice(1)}${params.toString()}`;
  window.history.replaceState(window.history.state, "", url);
}

function readHint(key: string): string | null {
  try {
    return validHintKey(key) ? readHintParams().get(key) : null;
  } catch {
    return null;
  }
}

function writeHint(key: string, value: string): void {
  try {
    if (!validHintKey(key) || !validHintUuid(value)) return;
    const params = readHintParams();
    params.set(key, value);
    replaceHintParams(params);
  } catch {
    // The server identity remains authoritative if the URL cannot retain a hint.
  }
}

function removeHint(key: string): void {
  try {
    if (!validHintKey(key)) return;
    const params = readHintParams();
    params.delete(key);
    replaceHintParams(params);
  } catch {
    // Reentry can continue from the server state without a local hint.
  }
}

function safeEpoch(epoch: number | null): epoch is number {
  return epoch !== null && Number.isSafeInteger(epoch) && epoch >= 0;
}

function errorCode(error: unknown): string {
  return error instanceof ApiProblem ? error.code : "REQUEST_FAILED";
}

function isCommitUnknown(error: unknown): boolean {
  return error instanceof ApiProblem && error.code === "PAPER_COMMIT_UNKNOWN";
}

function amount(value: string | null, fallback: string): string {
  return value ?? fallback;
}

function DefinitionList({ children }: { readonly children: ReactNode }) {
  return <dl className="definition-grid">{children}</dl>;
}

export function OwnerBetaPaperPanel({
  actualOrders = [],
  initialError = false,
  initialState,
  runs,
}: OwnerBetaPaperPanelProps) {
  const { locale } = useLocale();
  const router = useRouter();
  const t = paperDictionary[locale].ownerBeta;
  const [paperState, setPaperState] = useState(initialState);
  const [loadingState, setLoadingState] = useState(initialState === null && !initialError);
  const [panelError, setPanelError] = useState<PanelError | null>(
    initialError ? { code: "STATE_UNAVAILABLE", kind: "error" } : null,
  );
  const [preview, setPreview] = useState<OwnerBetaPaperPreviewModel | null>(null);
  const [appliedTargetId, setAppliedTargetId] = useState<string | null>(null);
  const [previewPolls, setPreviewPolls] = useState(0);
  const [selectedRunId, setSelectedRunId] = useState("");
  const [selectedTargetId, setSelectedTargetId] = useState("");
  const [sessionDetail, setSessionDetail] = useState<OwnerBetaPaperSessionDetailModel | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [detailError, setDetailError] = useState(false);
  const [detailRetry, setDetailRetry] = useState(0);
  const [submitting, setSubmitting] = useState(false);
  const [consentConfirmed, setConsentConfirmed] = useState(false);
  const [consentMessage, setConsentMessage] = useState(false);
  const [actionMessage, setActionMessage] = useState<"applied" | null>(null);
  const actionLock = useRef(false);
  const stateSequence = useRef(0);
  const detailSequence = useRef(0);
  const previewSequence = useRef(0);
  const alive = useRef(true);
  const stateRef = useRef(paperState);
  stateRef.current = paperState;

  const readPreview = useCallback(
    async (previewId: string): Promise<OwnerBetaPaperPreviewModel | null> => {
      const requestId = ++previewSequence.current;
      try {
        const response = await fetch(ownerBetaPaperPreviewPath(previewId), {
          cache: "no-store",
          credentials: "same-origin",
        });
        const persisted = await parseBrowserApiResponse(response, ownerBetaPaperPreviewSchema);
        return requestId === previewSequence.current && persisted.preview_id === previewId
          ? persisted
          : null;
      } catch {
        return null;
      }
    },
    [],
  );

  const matchingRuns = useMemo(
    () => runs.filter((run) => run.strategyConfigId === paperState?.strategy_config_id),
    [paperState?.strategy_config_id, runs],
  );
  const usableEpoch = safeEpoch(paperState?.control_epoch ?? null);
  const canManageConsent =
    paperState !== null &&
    paperState.state !== "DISABLED" &&
    paperState.account_id !== null &&
    usableEpoch;
  const canCreatePreview =
    paperState?.state === "ENABLED" &&
    paperState.consent_present &&
    paperState.account_id !== null &&
    paperState.strategy_config_id !== null &&
    usableEpoch;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      stateSequence.current += 1;
      detailSequence.current += 1;
      previewSequence.current += 1;
    };
  }, []);

  async function refreshState(): Promise<OwnerBetaPaperStateModel | null> {
    const requestId = ++stateSequence.current;
    setLoadingState(true);
    try {
      const response = await fetch(OWNER_BETA_PAPER_STATE_PATH, {
        cache: "no-store",
        credentials: "same-origin",
      });
      const next = await parseBrowserApiResponse(response, ownerBetaPaperStateSchema);
      if (alive.current && requestId === stateSequence.current) {
        setPaperState(next);
        setPanelError((current) => (current?.kind === "unknown-commit" ? current : null));
      }
      return next;
    } catch (error) {
      if (alive.current && requestId === stateSequence.current) {
        setPanelError({ code: errorCode(error), kind: "error" });
      }
      return null;
    } finally {
      if (alive.current && requestId === stateSequence.current) setLoadingState(false);
    }
  }

  useEffect(() => {
    if (paperState === null) return;
    if (paperState.state === "DISABLED") {
      setSelectedTargetId("");
      setSessionDetail(null);
      return;
    }
    setSelectedTargetId((current) =>
      current && paperState.sessions.some((session) => session.pending_target_id === current)
        ? current
        : (paperState.sessions[0]?.pending_target_id ?? ""),
    );
  }, [paperState]);

  useEffect(() => {
    const accountId = paperState?.account_id;
    if (
      paperState === null ||
      accountId === null ||
      accountId === undefined ||
      paperState.state === "DISABLED"
    ) {
      setAppliedTargetId(null);
      return;
    }
    const targetId = readHint(previewTargetHintKey(accountId));
    setAppliedTargetId(
      targetId !== null &&
        paperState.sessions.some((session) => session.pending_target_id === targetId)
        ? targetId
        : null,
    );
  }, [paperState]);

  useEffect(() => {
    if (!matchingRuns.some((run) => run.id === selectedRunId)) {
      setSelectedRunId(matchingRuns[0]?.id ?? "");
    }
  }, [matchingRuns, selectedRunId]);

  useEffect(() => {
    const accountId = paperState?.account_id;
    const expectedEpoch = paperState?.control_epoch;
    if (
      accountId === null ||
      accountId === undefined ||
      expectedEpoch === null ||
      expectedEpoch === undefined ||
      paperState?.state === "DISABLED"
    ) {
      previewSequence.current += 1;
      setPreview(null);
      const unknownAccountId = readHint(UNKNOWN_COMMIT_ACCOUNT_KEY);
      if (unknownAccountId !== null && readHint(unknownCommitHintKey(unknownAccountId)) !== null) {
        setPanelError({ code: "PAPER_COMMIT_UNKNOWN", kind: "unknown-commit" });
      }
      return;
    }
    const unresolvedAccountId = readHint(UNKNOWN_COMMIT_ACCOUNT_KEY);
    if (
      unresolvedAccountId !== null &&
      readHint(unknownCommitHintKey(unresolvedAccountId)) !== null
    ) {
      setPanelError({ code: "PAPER_COMMIT_UNKNOWN", kind: "unknown-commit" });
    }
    const key = previewHintKey(accountId);
    const previewId = readHint(key);
    if (previewId === null) return;
    let cancelled = false;
    void (async () => {
      const persisted = await readPreview(previewId);
      if (cancelled) return;
      if (
        persisted !== null &&
        readHint(key) === previewId &&
        persisted.account_id === accountId &&
        persisted.control_epoch === expectedEpoch
      ) {
        setPreview(persisted);
        setPreviewPolls(0);
        if (readHint(unknownCommitHintKey(accountId)) === previewId) {
          if (persisted.status === "APPLIED") {
            removeHint(unknownCommitHintKey(accountId));
            if (readHint(UNKNOWN_COMMIT_ACCOUNT_KEY) === accountId) {
              removeHint(UNKNOWN_COMMIT_ACCOUNT_KEY);
            }
            setPanelError(null);
          } else {
            setPanelError({ code: "PAPER_COMMIT_UNKNOWN", kind: "unknown-commit" });
          }
        }
      } else if (readHint(key) === previewId) {
        removeHint(key);
        setPanelError({ code: "PAPER_STALE", kind: "error" });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [paperState?.account_id, paperState?.control_epoch, paperState?.state, readPreview]);

  useEffect(() => {
    // A manual retry must re-read the same target even when its identity is unchanged.
    void detailRetry;
    if (paperState?.state === "DISABLED" || selectedTargetId === "") {
      setSessionDetail(null);
      setDetailError(false);
      return;
    }
    const requestId = ++detailSequence.current;
    let cancelled = false;
    setSessionDetail(null);
    setLoadingDetail(true);
    setDetailError(false);
    void (async () => {
      try {
        const response = await fetch(ownerBetaPaperSessionPath(selectedTargetId), {
          cache: "no-store",
          credentials: "same-origin",
        });
        const next = await parseBrowserApiResponse(response, ownerBetaPaperSessionDetailSchema);
        if (
          !cancelled &&
          requestId === detailSequence.current &&
          next.pending_target_id === selectedTargetId
        ) {
          setSessionDetail(next);
        } else if (!cancelled && requestId === detailSequence.current) {
          setDetailError(true);
        }
      } catch {
        if (!cancelled && requestId === detailSequence.current) setDetailError(true);
      } finally {
        if (!cancelled && requestId === detailSequence.current) setLoadingDetail(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [detailRetry, paperState?.state, selectedTargetId]);

  useEffect(() => {
    if (
      preview === null ||
      paperState?.state === "DISABLED" ||
      (preview.status !== "PENDING" && preview.status !== "RUNNING") ||
      previewPolls >= MAX_PREVIEW_POLLS
    ) {
      return;
    }
    const delay = POLL_DELAYS_MS[Math.min(previewPolls, POLL_DELAYS_MS.length - 1)] ?? 8_000;
    let cancelled = false;
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const next = await readPreview(preview.preview_id);
          if (
            !cancelled &&
            next !== null &&
            next.preview_id === preview.preview_id &&
            next.account_id === preview.account_id &&
            next.control_epoch === paperState?.control_epoch
          ) {
            setPreview(next);
            setPreviewPolls((attempt) => attempt + 1);
          } else if (!cancelled) {
            setPanelError({ code: "PAPER_STALE", kind: "error" });
          }
        } catch (error) {
          if (!cancelled) setPanelError({ code: errorCode(error), kind: "error" });
        }
      })();
    }, delay);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [paperState?.control_epoch, paperState?.state, preview, previewPolls, readPreview]);

  async function mutateConsent(optIn: boolean): Promise<void> {
    if (
      actionLock.current ||
      !canManageConsent ||
      paperState === null ||
      !safeEpoch(paperState.control_epoch)
    ) {
      return;
    }
    actionLock.current = true;
    setSubmitting(true);
    setPanelError(null);
    setConsentMessage(false);
    try {
      const body = ownerBetaPaperConsentBodySchema.parse({
        expected_epoch: paperState.control_epoch,
        opt_in: optIn,
      });
      const response = await mutateWithCsrf(OWNER_BETA_PAPER_CONSENT_PATH, {
        json: body,
        method: "POST",
      });
      const saved = await parseBrowserApiResponse(response, ownerBetaPaperConsentSchema);
      if (alive.current && saved.account_id === paperState.account_id) {
        setConsentMessage(true);
        await refreshState();
      }
    } catch (error) {
      if (alive.current) setPanelError({ code: errorCode(error), kind: "error" });
    } finally {
      actionLock.current = false;
      if (alive.current) setSubmitting(false);
    }
  }

  async function createPreview(): Promise<void> {
    if (
      actionLock.current ||
      panelError?.kind === "unknown-commit" ||
      !canCreatePreview ||
      paperState === null ||
      paperState.account_id === null ||
      paperState.strategy_config_id === null ||
      !safeEpoch(paperState.control_epoch) ||
      selectedRunId === ""
    ) {
      return;
    }
    const actionEpoch = paperState.control_epoch;
    if (
      readHint(UNKNOWN_COMMIT_ACCOUNT_KEY) !== null ||
      readHint(unknownCommitHintKey(paperState.account_id)) !== null
    ) {
      setPanelError({ code: "PAPER_COMMIT_UNKNOWN", kind: "unknown-commit" });
      return;
    }
    const selectedRun = matchingRuns.find((run) => run.id === selectedRunId);
    if (selectedRun === undefined) return;
    previewSequence.current += 1;
    actionLock.current = true;
    setSubmitting(true);
    setPanelError(null);
    setActionMessage(null);
    try {
      const body = ownerBetaPaperPreviewBodySchema.parse({
        expected_epoch: actionEpoch,
        recommendation_run_id: selectedRun.id,
      });
      const response = await mutateWithCsrf(OWNER_BETA_PAPER_PREVIEWS_PATH, {
        json: body,
        method: "POST",
      });
      const created = await parseBrowserApiResponse(response, ownerBetaPaperPreviewSchema);
      if (
        !alive.current ||
        created.account_id !== paperState.account_id ||
        created.lineage.recommendation_run_id !== selectedRun.id
      ) {
        return;
      }
      writeHint(previewHintKey(created.account_id), created.preview_id);
      previewSequence.current += 1;
      setPreview(created);
      setPreviewPolls(0);
      if (
        stateRef.current?.account_id !== created.account_id ||
        stateRef.current?.control_epoch !== actionEpoch
      ) {
        setPanelError({ code: "PAPER_STALE", kind: "error" });
      }
    } catch (error) {
      if (alive.current) setPanelError({ code: errorCode(error), kind: "error" });
    } finally {
      actionLock.current = false;
      if (alive.current) setSubmitting(false);
    }
  }

  async function refreshPreview(): Promise<void> {
    if (
      preview === null ||
      paperState === null ||
      paperState.state === "DISABLED" ||
      actionLock.current
    ) {
      return;
    }
    const persisted = await readPreview(preview.preview_id);
    if (!alive.current) return;
    if (
      persisted === null ||
      persisted.account_id !== paperState.account_id ||
      persisted.control_epoch !== paperState.control_epoch ||
      stateRef.current?.state === "DISABLED" ||
      stateRef.current?.account_id !== persisted.account_id ||
      stateRef.current?.control_epoch !== persisted.control_epoch
    ) {
      await refreshState();
      setPanelError({ code: "PAPER_STALE", kind: "error" });
      return;
    }
    setPreview(persisted);
    setPreviewPolls(0);
    setPanelError(null);
  }

  async function applyPreview(): Promise<void> {
    if (
      actionLock.current ||
      panelError?.kind === "unknown-commit" ||
      preview === null ||
      paperState === null ||
      preview.status !== "READY" ||
      preview.preview_token === null ||
      preview.account_id !== paperState.account_id ||
      preview.control_epoch !== paperState.control_epoch ||
      !safeEpoch(preview.control_epoch) ||
      paperState.state !== "ENABLED"
    ) {
      return;
    }
    if (
      readHint(UNKNOWN_COMMIT_ACCOUNT_KEY) !== null ||
      readHint(unknownCommitHintKey(paperState.account_id)) !== null
    ) {
      setPanelError({ code: "PAPER_COMMIT_UNKNOWN", kind: "unknown-commit" });
      return;
    }
    const persistedIdentity = {
      accountId: preview.account_id,
      epoch: preview.control_epoch,
      previewId: preview.preview_id,
      token: preview.preview_token,
    };
    previewSequence.current += 1;
    actionLock.current = true;
    setSubmitting(true);
    setPanelError(null);
    setActionMessage(null);
    try {
      const body = ownerBetaPaperApplyBodySchema.parse({
        expected_epoch: persistedIdentity.epoch,
        preview_token: persistedIdentity.token,
      });
      const response = await mutateWithCsrf(ownerBetaPaperApplyPath(persistedIdentity.previewId), {
        json: body,
        method: "POST",
      });
      const applied: OwnerBetaPaperApplyModel = await parseBrowserApiResponse(
        response,
        ownerBetaPaperApplySchema,
      );
      if (alive.current && applied.preview_id === persistedIdentity.previewId) {
        setActionMessage("applied");
        setSelectedTargetId(applied.pending_target_id);
        removeHint(unknownCommitHintKey(persistedIdentity.accountId));
        if (readHint(UNKNOWN_COMMIT_ACCOUNT_KEY) === persistedIdentity.accountId) {
          removeHint(UNKNOWN_COMMIT_ACCOUNT_KEY);
        }
        writeHint(previewTargetHintKey(persistedIdentity.accountId), applied.pending_target_id);
        setAppliedTargetId(applied.pending_target_id);
        const nextPreview = await readPreview(persistedIdentity.previewId);
        if (!alive.current) return;
        let staleResult =
          applied.control_epoch !== persistedIdentity.epoch ||
          applied.effective_session !== preview.effective_session ||
          stateRef.current?.account_id !== persistedIdentity.accountId ||
          stateRef.current?.control_epoch !== persistedIdentity.epoch;
        if (
          nextPreview !== null &&
          nextPreview.account_id === persistedIdentity.accountId &&
          nextPreview.control_epoch === persistedIdentity.epoch
        ) {
          setPreview(nextPreview);
        } else {
          staleResult = true;
        }
        await refreshState();
        if (alive.current && staleResult) setPanelError({ code: "PAPER_STALE", kind: "error" });
        router.refresh();
      } else if (alive.current) {
        await refreshState();
        setPanelError({ code: "PAPER_STALE", kind: "error" });
        router.refresh();
      }
    } catch (error) {
      if (isCommitUnknown(error)) {
        // The API contract says commit outcome is uncertain. Only read back;
        // never resend an economic action after this response.
        writeHint(unknownCommitHintKey(persistedIdentity.accountId), persistedIdentity.previewId);
        writeHint(UNKNOWN_COMMIT_ACCOUNT_KEY, persistedIdentity.accountId);
        const readbackState = await refreshState();
        const persisted =
          readbackState !== null && readbackState.state !== "DISABLED"
            ? await readPreview(persistedIdentity.previewId)
            : null;
        if (
          alive.current &&
          readbackState?.state !== "DISABLED" &&
          persisted !== null &&
          persisted.account_id === persistedIdentity.accountId &&
          persisted.control_epoch === persistedIdentity.epoch
        ) {
          setPreview(persisted);
        }
        if (alive.current) {
          setPanelError({ code: "PAPER_COMMIT_UNKNOWN", kind: "unknown-commit" });
        }
      } else if (alive.current) {
        setPanelError({ code: errorCode(error), kind: "error" });
      }
    } finally {
      actionLock.current = false;
      if (alive.current) setSubmitting(false);
    }
  }

  const matchingOrders = useMemo(() => {
    if (sessionDetail === null) return [];
    return sessionDetail.order_references.map((reference) => ({
      reference,
      order: actualOrders.find(
        (candidate) =>
          candidate.id === reference.order_ref || candidate.order_ref === reference.order_ref,
      ),
    }));
  }, [actualOrders, sessionDetail]);

  const errorText =
    panelError?.kind === "unknown-commit"
      ? t.unknownCommit
      : panelError === null
        ? t.unavailableMessage
        : t.errorMessage(panelError.code);
  const sessionState = paperState?.state;

  async function refreshOwnerStatus(): Promise<void> {
    const accountId = stateRef.current?.account_id;
    const unresolvedAccountId = readHint(UNKNOWN_COMMIT_ACCOUNT_KEY) ?? accountId ?? null;
    const unresolvedPreviewId = unresolvedAccountId
      ? readHint(unknownCommitHintKey(unresolvedAccountId))
      : null;
    const refreshedState = await refreshState();
    if (!alive.current) return;
    if (unresolvedAccountId !== null && unresolvedPreviewId !== null) {
      const persisted =
        refreshedState !== null && refreshedState.state !== "DISABLED"
          ? await readPreview(unresolvedPreviewId)
          : null;
      if (
        refreshedState !== null &&
        refreshedState.state !== "DISABLED" &&
        refreshedState.account_id === unresolvedAccountId &&
        persisted !== null &&
        persisted.account_id === unresolvedAccountId
      ) {
        setPreview(persisted);
        // READY can still be visible while the original transaction is committing.
        // Only its persisted applied result resolves the uncertain action here.
        if (persisted.status === "APPLIED") {
          removeHint(unknownCommitHintKey(unresolvedAccountId));
          if (readHint(UNKNOWN_COMMIT_ACCOUNT_KEY) === unresolvedAccountId) {
            removeHint(UNKNOWN_COMMIT_ACCOUNT_KEY);
          }
          setPanelError(null);
        } else {
          setPanelError({ code: "PAPER_COMMIT_UNKNOWN", kind: "unknown-commit" });
        }
      } else {
        setPanelError({ code: "PAPER_COMMIT_UNKNOWN", kind: "unknown-commit" });
      }
    }
    router.refresh();
  }

  return (
    <section aria-labelledby="owner-beta-paper-title" className="workflow-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.eyebrow}</p>
          <h2 id="owner-beta-paper-title">{t.title}</h2>
        </div>
        <button
          className="secondary-action"
          disabled={loadingState || submitting}
          onClick={() => void refreshOwnerStatus()}
          type="button"
        >
          {t.refresh}
        </button>
      </div>

      {paperState === null ? (
        <div className="state-panel" data-kind="error" role="alert">
          <p>{errorText}</p>
          {panelError === null ? null : <code>{panelError.code}</code>}
        </div>
      ) : paperState.state === "DISABLED" ? (
        <>
          <div className="state-panel" data-kind="blocked" role="status">
            <strong>{t.disabledTitle}</strong>
            <p>{t.disabledMessage}</p>
          </div>
          {panelError === null ? null : (
            <div className="state-panel" data-kind="error" role="alert">
              <p>{errorText}</p>
              <code>{panelError.code}</code>
            </div>
          )}
        </>
      ) : (
        <>
          <div className="state-panel" data-kind={sessionState === "BLOCKED" ? "blocked" : "info"}>
            <strong>
              {t.stateLabel}: {t.stateName(paperState.state)}
            </strong>
            <p>{t.stateDescription(paperState.state)}</p>
            {paperState.strategy_id === null ? null : (
              <p>
                {paperState.strategy_id}
                {paperState.strategy_version === null ? "" : `@${paperState.strategy_version}`}
              </p>
            )}
          </div>
          <p className="supporting-copy">{t.comparisonSummary}</p>
          <p className="supporting-copy">{t.noBacktestPerformance}</p>

          <section aria-labelledby="owner-beta-paper-consent-title">
            <h3 id="owner-beta-paper-consent-title">{t.consentTitle}</h3>
            <p className="supporting-copy">{t.consentMessage}</p>
            <p className="supporting-copy">{t.activationNote}</p>
            <p className="supporting-copy">{t.autoBindingNote}</p>
            {canManageConsent ? (
              <div className="workflow-form">
                {!paperState.consent_present ? (
                  <label className="form-field">
                    <input
                      checked={consentConfirmed}
                      onChange={(event) => setConsentConfirmed(event.currentTarget.checked)}
                      type="checkbox"
                    />
                    <span>{t.consentCheckbox}</span>
                  </label>
                ) : null}
                <button
                  className={paperState.consent_present ? "secondary-action" : "primary-action"}
                  disabled={
                    submitting || loadingState || (!paperState.consent_present && !consentConfirmed)
                  }
                  onClick={() => void mutateConsent(!paperState.consent_present)}
                  type="button"
                >
                  {paperState.consent_present ? t.consentOutButton : t.consentInButton}
                </button>
                {consentMessage ? (
                  <p className="form-result" role="status">
                    {t.consentSaved}
                  </p>
                ) : null}
              </div>
            ) : (
              <p className="supporting-copy">{t.stateDescription(paperState.state)}</p>
            )}
          </section>

          {panelError === null ? null : (
            <div className="state-panel" data-kind="error" role="alert">
              <p>{errorText}</p>
              <code>{panelError.code}</code>
            </div>
          )}

          {canCreatePreview ? (
            <section aria-labelledby="owner-beta-paper-preview-create-title">
              <h3 id="owner-beta-paper-preview-create-title">{t.previewTitle}</h3>
              {matchingRuns.length === 0 ? (
                <p className="supporting-copy">{t.noRuns}</p>
              ) : (
                <form
                  className="workflow-form"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void createPreview();
                  }}
                >
                  <label className="form-field">
                    <span>{t.runLabel}</span>
                    <select
                      onChange={(event) => setSelectedRunId(event.currentTarget.value)}
                      value={
                        matchingRuns.some((run) => run.id === selectedRunId)
                          ? selectedRunId
                          : (matchingRuns[0]?.id ?? "")
                      }
                    >
                      {matchingRuns.map((run) => (
                        <option key={run.id} value={run.id}>
                          {run.asOf} · {run.strategyLabel}
                        </option>
                      ))}
                    </select>
                  </label>
                  <button
                    className="primary-action"
                    disabled={submitting || loadingState || panelError?.kind === "unknown-commit"}
                    type="submit"
                  >
                    {submitting ? t.previewCreating : t.previewButton}
                  </button>
                </form>
              )}
            </section>
          ) : null}

          {preview === null ? null : (
            <OwnerBetaPreviewView
              actionMessage={actionMessage}
              applyDisabled={
                submitting ||
                loadingState ||
                panelError?.kind === "unknown-commit" ||
                paperState.state !== "ENABLED" ||
                preview.account_id !== paperState.account_id ||
                preview.control_epoch !== paperState.control_epoch
              }
              onApply={() => void applyPreview()}
              onRefresh={() => void refreshPreview()}
              canRetryPending={panelError?.kind === "error"}
              preview={preview}
              previewPolls={previewPolls}
              t={t}
              busy={submitting}
            />
          )}

          <section aria-labelledby="owner-beta-paper-sessions-title">
            <h3 id="owner-beta-paper-sessions-title">{t.sessionsTitle}</h3>
            {paperState.sessions.length === 0 ? (
              <p className="supporting-copy">{t.noSessions}</p>
            ) : (
              <table>
                <thead>
                  <tr>
                    <th scope="col">{t.sessionEffective}</th>
                    <th scope="col">{t.stateLabel}</th>
                    <th scope="col">{t.settledAt}</th>
                    <th scope="col">{t.valuedAt}</th>
                    <th scope="col">{t.reasonCode}</th>
                  </tr>
                </thead>
                <tbody>
                  {paperState.sessions.map((session) => (
                    <tr key={session.pending_target_id}>
                      <th scope="row">
                        <button
                          aria-pressed={selectedTargetId === session.pending_target_id}
                          className="text-action"
                          onClick={() => setSelectedTargetId(session.pending_target_id)}
                          type="button"
                        >
                          {session.effective_session}
                        </button>
                      </th>
                      <td>{t.sessionState(session.state)}</td>
                      <td>{session.settled_at ?? t.noValue}</td>
                      <td>{session.valued_at ?? t.noValue}</td>
                      <td>{session.reason_code ?? t.noValue}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>

          {loadingDetail ? (
            <p className="supporting-copy" role="status">
              {t.previewWaiting}
            </p>
          ) : null}
          {detailError ? (
            <div className="state-panel" data-kind="error" role="alert">
              <p>{t.unavailableMessage}</p>
              <button
                className="secondary-action"
                onClick={() => setDetailRetry((value) => value + 1)}
                type="button"
              >
                {t.retry}
              </button>
            </div>
          ) : null}
          {sessionDetail === null ? null : (
            <OwnerBetaSessionView
              detail={sessionDetail}
              appliedTargetId={appliedTargetId}
              matchingOrders={matchingOrders}
              preview={preview}
              t={t}
            />
          )}
        </>
      )}
    </section>
  );
}

function OwnerBetaPreviewView({
  actionMessage,
  applyDisabled,
  busy,
  canRetryPending,
  onApply,
  onRefresh,
  preview,
  previewPolls,
  t,
}: {
  readonly actionMessage: "applied" | null;
  readonly applyDisabled: boolean;
  readonly busy: boolean;
  readonly canRetryPending: boolean;
  readonly onApply: () => void;
  readonly onRefresh: () => void;
  readonly preview: OwnerBetaPaperPreviewModel;
  readonly previewPolls: number;
  readonly t: (typeof paperDictionary)["en"]["ownerBeta"];
}) {
  if (preview.status === "PENDING" || preview.status === "RUNNING") {
    return (
      <div className="state-panel" data-kind="info" role="status">
        <strong>
          {t.previewStatus}: {t.previewState(preview.status)}
        </strong>
        <p>{previewPolls >= MAX_PREVIEW_POLLS ? t.previewTimedOut : t.previewWaiting}</p>
        {previewPolls >= MAX_PREVIEW_POLLS || canRetryPending ? (
          <button className="secondary-action" disabled={busy} onClick={onRefresh} type="button">
            {t.retry}
          </button>
        ) : null}
      </div>
    );
  }
  if (preview.status === "FAILED") {
    return (
      <div className="state-panel" data-kind="blocked" role="alert">
        <strong>{t.previewFailed}</strong>
        {preview.error === null ? null : (
          <p>
            {t.errorMessage(preview.error.code)} <code>{preview.error.code}</code>
          </p>
        )}
      </div>
    );
  }
  const result = preview.result;
  return (
    <section aria-labelledby="owner-beta-paper-preview-result-title" className="report-section">
      <h3 id="owner-beta-paper-preview-result-title">{t.previewTitle}</h3>
      {actionMessage === "applied" || preview.status === "APPLIED" ? (
        <p className="form-result" role="status">
          {t.previewApplied}
        </p>
      ) : null}
      <p className="supporting-copy">
        {t.previewStatus}: {t.previewState(preview.status)}
      </p>
      <DefinitionList>
        <dt>{t.previewSignal}</dt>
        <dd>{preview.signal_date}</dd>
        <dt>{t.previewEffective}</dt>
        <dd>{preview.effective_session}</dd>
        <dt>{t.costProfile}</dt>
        <dd>{preview.cost_profile_id ?? t.noValue}</dd>
        <dt>{t.costProfileVersion}</dt>
        <dd>{preview.cost_profile_version ?? t.noValue}</dd>
      </DefinitionList>
      <p className="supporting-copy">
        {t.signalDateHelp} {t.effectiveDateHelp}
      </p>
      {result === null ? null : (
        <>
          <h4>{t.previewAmount}</h4>
          <DefinitionList>
            <dt>{t.equity}</dt>
            <dd>{result.equity}</dd>
            <dt>{t.cashBefore}</dt>
            <dd>{result.cash_before}</dd>
            <dt>{t.availableCash}</dt>
            <dd>{result.available_cash}</dd>
            <dt>{t.leftoverCash}</dt>
            <dd>{result.leftover_cash}</dd>
            <dt>{t.buyNotional}</dt>
            <dd>{result.buy_notional}</dd>
            <dt>{t.sellNotional}</dt>
            <dd>{result.sell_notional}</dd>
            <dt>{t.explicitFees}</dt>
            <dd>{result.explicit_fees}</dd>
            <dt>{t.informationalSlippage}</dt>
            <dd>{result.informational_slippage}</dd>
            <dt>{t.previewOrders}</dt>
            <dd>{t.planStatus(result.plan_status)}</dd>
          </DefinitionList>
          <p className="supporting-copy">{t.noBacktestPerformance}</p>
          <table>
            <caption>{t.previewOrders}</caption>
            <thead>
              <tr>
                <th scope="col">{t.columnSide}</th>
                <th scope="col">{t.columnQuantity}</th>
                <th scope="col">{t.previewOrderPrice}</th>
                <th scope="col">{t.priceObservation}</th>
              </tr>
            </thead>
            <tbody>
              {result.orders.map((order) => {
                const observation = result.price_observations.find(
                  (item) => item.instrument_id === order.instrument_id,
                );
                return (
                  <tr key={`${order.instrument_id}-${order.side}`}>
                    <th scope="row">
                      {order.instrument_id} · {order.side}
                    </th>
                    <td>{order.quantity}</td>
                    <td>
                      {order.raw_price} → {order.estimated_execution_price}
                    </td>
                    <td>
                      {observation === undefined
                        ? t.noValue
                        : `${observation.session_date} · ${observation.observed_at}`}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </>
      )}
      {preview.status === "READY" ? (
        <button
          className="primary-action"
          disabled={applyDisabled || result === null || preview.preview_token === null}
          onClick={onApply}
          type="button"
        >
          {busy ? t.previewApplying : t.previewApplyButton}
        </button>
      ) : null}
    </section>
  );
}

function OwnerBetaSessionView({
  appliedTargetId,
  detail,
  matchingOrders,
  preview,
  t,
}: {
  readonly detail: OwnerBetaPaperSessionDetailModel;
  readonly appliedTargetId: string | null;
  readonly matchingOrders: readonly {
    readonly order: PaperOrderModel | undefined;
    readonly reference: OwnerBetaPaperSessionDetailModel["order_references"][number];
  }[];
  readonly preview: OwnerBetaPaperPreviewModel | null;
  readonly t: (typeof paperDictionary)["en"]["ownerBeta"];
}) {
  const matchingPreview =
    preview?.status === "APPLIED" &&
    appliedTargetId === detail.pending_target_id &&
    preview.effective_session === detail.effective_session
      ? preview
      : null;
  const proposedOrders = matchingPreview?.result?.orders ?? [];
  const actual = matchingOrders.reduce<PaperOrderModel[]>((unique, item) => {
    if (item.order !== undefined && !unique.some((order) => order.id === item.order?.id)) {
      unique.push(item.order);
    }
    return unique;
  }, []);
  return (
    <section aria-labelledby="owner-beta-paper-session-detail-title" className="report-section">
      <h3 id="owner-beta-paper-session-detail-title">{t.sessionDetails}</h3>
      <p className="supporting-copy">
        {t.sessionState(detail.state)} · {t.processingOrigin(detail.processing_origin)}
      </p>
      <DefinitionList>
        <dt>{t.sessionSignal}</dt>
        <dd>{detail.signal_date}</dd>
        <dt>{t.sessionEffective}</dt>
        <dd>{detail.effective_session}</dd>
        <dt>{t.sourceRetrievedAt}</dt>
        <dd>{detail.source_original_retrieved_at ?? t.noValue}</dd>
        <dt>{t.admittedAt}</dt>
        <dd>{detail.input_admitted_at ?? t.noValue}</dd>
        <dt>{t.usableAt}</dt>
        <dd>{detail.usable_at ?? t.noValue}</dd>
        <dt>{t.settledAt}</dt>
        <dd>{detail.settled_at ?? t.noValue}</dd>
        <dt>{t.valuedAt}</dt>
        <dd>{detail.valued_at ?? t.noValue}</dd>
        <dt>{t.settlementKind}</dt>
        <dd>{t.settlementKindLabel(detail.settlement_kind)}</dd>
        <dt>{t.reasonCode}</dt>
        <dd>{detail.reason_code ?? t.noValue}</dd>
      </DefinitionList>
      {(detail.state === "SETTLED_TRADE" || detail.state === "SETTLED_NO_TRADE") &&
      detail.valued_at === null ? (
        <p className="supporting-copy" role="status">
          {t.valuationPending}
        </p>
      ) : null}

      <h4>{t.actualAmounts}</h4>
      <DefinitionList>
        <dt>{t.cashAfter}</dt>
        <dd>{amount(detail.amounts.cash_after, t.noValue)}</dd>
        <dt>{t.valuationCash}</dt>
        <dd>{amount(detail.amounts.valuation_cash, t.noValue)}</dd>
        <dt>{t.positionsValue}</dt>
        <dd>{amount(detail.amounts.positions_value, t.noValue)}</dd>
        <dt>{t.equity}</dt>
        <dd>{amount(detail.amounts.equity, t.noValue)}</dd>
      </DefinitionList>

      <h4>{t.actualDifferenceTitle}</h4>
      {detail.order_references.length === 0 ? (
        <p className="supporting-copy">{t.noActualOrders}</p>
      ) : (
        <>
          <p className="supporting-copy">{t.actualOrderCount(detail.order_references.length)}</p>
          <p className="supporting-copy">
            {t.actualOrderDetailsCount(actual.length, detail.order_references.length)}
          </p>
          {actual.length === 0 ? null : (
            <table>
              <caption>{t.orderReferences}</caption>
              <thead>
                <tr>
                  <th scope="col">{t.columnSide}</th>
                  <th scope="col">{t.actualQuantity}</th>
                  <th scope="col">{t.actualPrice}</th>
                  <th scope="col">{t.stateLabel}</th>
                </tr>
              </thead>
              <tbody>
                {actual.map((order) => (
                  <tr key={order.id}>
                    <th scope="row">{`${order.instrument_id} · ${order.side}`}</th>
                    <td>{order.quantity}</td>
                    <td>{order.price ?? t.noValue}</td>
                    <td>{order.status}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {proposedOrders.length > 0 ? (
            <>
              <p className="supporting-copy">
                {t.actualDifferenceCount(proposedOrders.length, actual.length)}
              </p>
              <p className="supporting-copy">{t.decimalExactNote}</p>
              <table>
                <caption>{t.actualDifferenceTitle}</caption>
                <thead>
                  <tr>
                    <th scope="col">{t.columnSide}</th>
                    <th scope="col">{t.proposedQuantity}</th>
                    <th scope="col">{t.previewOrderPrice}</th>
                    <th scope="col">{t.actualQuantity}</th>
                    <th scope="col">{t.actualPrice}</th>
                  </tr>
                </thead>
                <tbody>
                  {proposedOrders.map((proposed) => {
                    const matchingActual = actual.filter(
                      (order) =>
                        order.instrument_id === proposed.instrument_id &&
                        order.side === proposed.side,
                    );
                    const actualQuantities = matchingActual.map((order) => order.quantity);
                    const actualPrices = matchingActual.map((order) => order.price ?? t.noValue);
                    return (
                      <tr key={`${proposed.instrument_id}-${proposed.side}`}>
                        <th scope="row">
                          {proposed.instrument_id} · {proposed.side}
                        </th>
                        <td>{proposed.quantity}</td>
                        <td>
                          {proposed.raw_price} → {proposed.estimated_execution_price}
                        </td>
                        <td>
                          {actualQuantities.length === 0 ? t.noValue : actualQuantities.join(", ")}
                        </td>
                        <td>{actualPrices.length === 0 ? t.noValue : actualPrices.join(", ")}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </>
          ) : null}
        </>
      )}
      <h4>
        {t.comparison}: {t.comparisonDisposition(detail.comparison.disposition)}
      </h4>
      <p className="supporting-copy">{detail.comparison.reason}</p>
      <p className="supporting-copy">{t.noBacktestPerformance}</p>
    </section>
  );
}
