import type { LocaleDictionary } from "@/lib/i18n/locale";

export type LiveDictionary = {
  readonly columnAccount: string;
  readonly columnConnection: string;
  readonly columnCredentialLocations: string;
  readonly columnProfile: string;
  readonly configLoadFailedMessage: string;
  readonly connectionsCaption: string;
  readonly connectionsTitle: string;
  readonly credentialsFootnote: string;
  readonly disengageAriaLabel: string;
  readonly disengageButtonDisengage: string;
  readonly disengageButtonDisengaging: string;
  readonly disengagedMessage: string;
  readonly disengagedStatus: string;
  readonly disengagingSupportingCopy: string;
  readonly engageAriaLabel: string;
  readonly engageButtonEngage: string;
  readonly engageButtonEngaging: string;
  readonly engagedMessage: string;
  readonly engagedStatus: string;
  readonly engagingSupportingCopy: string;
  readonly freshAuthRequiredTitle: string;
  readonly keyLabel: (ref: string) => string;
  readonly killSwitchTitle: string;
  readonly liveProfileLabel: string;
  readonly mockProfileLabel: string;
  readonly noConnectionMessage: string;
  readonly notAvailableMessage: string;
  readonly pageDescription: string;
  readonly pageTitle: string;
  readonly reasonForDisengagingLabel: string;
  readonly safetyEyebrow: string;
  readonly secretLabel: (ref: string) => string;
  readonly unavailableTitle: string;
};

export const liveDictionary: LocaleDictionary<LiveDictionary> = {
  en: {
    columnAccount: "Account",
    columnConnection: "Connection",
    columnCredentialLocations: "Credential locations",
    columnProfile: "Profile",
    configLoadFailedMessage:
      "Live configuration could not be loaded. The kill switch remains engaged until this resolves.",
    connectionsCaption: "Configured broker connections",
    connectionsTitle: "Broker connections",
    credentialsFootnote:
      "Credentials are shown as locations, never values. The server stores a reference to where each credential lives and has no field capable of holding the credential itself.",
    disengageAriaLabel: "Disengage kill switch",
    disengageButtonDisengage: "Disengage kill switch",
    disengageButtonDisengaging: "Disengaging",
    disengagedMessage: "Kill switch disengaged. Live nodes may now start.",
    disengagedStatus: "Disengaged — Live may run",
    disengagingSupportingCopy:
      "Disengaging permits Live nodes to start and place real orders. The reason is recorded in the audit trail.",
    engageAriaLabel: "Engage kill switch",
    engageButtonEngage: "Engage kill switch",
    engageButtonEngaging: "Engaging",
    engagedMessage: "Kill switch engaged. No Live node can start.",
    engagedStatus: "Engaged — Live is stopped",
    engagingSupportingCopy:
      "Engaging stops Live immediately. No reason is required — refusing to stop trading because an operator has not explained themselves would be the wrong trade in the one moment it matters most.",
    freshAuthRequiredTitle: "Fresh authentication required",
    keyLabel: (ref) => `key: ${ref}`,
    killSwitchTitle: "Kill switch",
    liveProfileLabel: "LIVE — places real orders",
    mockProfileLabel: "Mock — simulated",
    noConnectionMessage:
      "No broker connection is configured. Live trading cannot start until one exists.",
    notAvailableMessage: "Live controls are not available to this session.",
    pageDescription:
      "Owner-only broker connections, node lifecycle, and the Live kill switch. Every action requires a fresh multi-factor authentication.",
    pageTitle: "Live controls",
    reasonForDisengagingLabel: "Reason for disengaging",
    safetyEyebrow: "Safety",
    secretLabel: (ref) => `secret: ${ref}`,
    unavailableTitle: "Live controls unavailable",
  },
  ko: {
    columnAccount: "계좌",
    columnConnection: "연결",
    columnCredentialLocations: "자격 증명 위치",
    columnProfile: "프로필",
    configLoadFailedMessage:
      "실거래 구성을 불러오지 못했습니다. 문제가 해결될 때까지 킬 스위치는 작동 상태로 유지됩니다.",
    connectionsCaption: "구성된 브로커 연결",
    connectionsTitle: "브로커 연결",
    credentialsFootnote:
      "자격 증명은 값이 아닌 위치로만 표시됩니다. 서버는 각 자격 증명이 저장된 위치에 대한 참조만 보관하며, 자격 증명 자체를 담을 수 있는 필드는 존재하지 않습니다.",
    disengageAriaLabel: "킬 스위치 해제",
    disengageButtonDisengage: "킬 스위치 해제",
    disengageButtonDisengaging: "해제 중",
    disengagedMessage: "킬 스위치가 해제되었습니다. 이제 실거래 노드를 시작할 수 있습니다.",
    disengagedStatus: "해제됨 — 실거래가 실행될 수 있습니다",
    disengagingSupportingCopy:
      "킬 스위치를 해제하면 실거래 노드가 시작되어 실제 주문을 낼 수 있습니다. 해제 사유는 감사 기록에 남습니다.",
    engageAriaLabel: "킬 스위치 작동",
    engageButtonEngage: "킬 스위치 작동",
    engageButtonEngaging: "작동 중",
    engagedMessage: "킬 스위치가 작동되었습니다. 실거래 노드를 시작할 수 없습니다.",
    engagedStatus: "작동 중 — 실거래가 중지되었습니다",
    engagingSupportingCopy:
      "킬 스위치를 작동하면 실거래가 즉시 중지됩니다. 사유는 필요하지 않습니다 — 운영자가 설명하지 않았다는 이유로 거래 중지를 거부하는 것은, 그것이 가장 중요한 바로 그 순간에 잘못된 판단이 될 것입니다.",
    freshAuthRequiredTitle: "최신 인증이 필요합니다",
    keyLabel: (ref) => `키: ${ref}`,
    killSwitchTitle: "킬 스위치",
    liveProfileLabel: "실거래 — 실제 주문을 냅니다",
    mockProfileLabel: "모의 — 시뮬레이션",
    noConnectionMessage:
      "구성된 브로커 연결이 없습니다. 연결이 존재해야 실거래를 시작할 수 있습니다.",
    notAvailableMessage: "이 세션에서는 실거래 제어를 사용할 수 없습니다.",
    pageDescription:
      "오너 전용 브로커 연결, 노드 수명 주기, 그리고 실거래 킬 스위치입니다. 모든 작업에는 최신 다중 인증이 필요합니다.",
    pageTitle: "실거래 제어",
    reasonForDisengagingLabel: "해제 사유",
    safetyEyebrow: "안전",
    secretLabel: (ref) => `시크릿: ${ref}`,
    unavailableTitle: "실거래 제어를 사용할 수 없습니다",
  },
};
