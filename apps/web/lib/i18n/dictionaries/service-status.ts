import type { Locale } from "@/lib/i18n/locale";
import type {
  ServiceStatusExpectedMode,
  ServiceStatusId,
  ServiceStatusKind,
  ServiceStatusProcessState,
  ServiceStatusReasonGroup,
  ServiceStatusState,
  ServiceStatusWorkState,
} from "@/lib/operations/service-status-contract";

export type ServiceStatusDictionary = {
  readonly heading: string;
  readonly disclosureLabel: string;
  readonly tableCaption: string;
  readonly serviceHeading: string;
  readonly statusHeading: string;
  readonly processHeading: string;
  readonly workHeading: string;
  readonly detailsHeading: string;
  readonly observedLabel: string;
  readonly observedAt: string;
  readonly lastRecorded: string;
  readonly loading: string;
  readonly unavailable: string;
  readonly rateLimited: string;
  readonly denied: string;
  readonly noDetails: string;
  readonly staleEvidence: string;
  readonly policyVerified: string;
  readonly policyUnavailable: string;
  readonly policyMismatch: string;
  readonly policyLabel: string;
  readonly countSummary: (count: number) => string;
  readonly sharedQueue: string;
  readonly noMetrics: string;
  readonly metricLabels: Readonly<Record<keyof ServiceStatusMetricDictionary, string>>;
  readonly statuses: Readonly<Record<ServiceStatusState, string>>;
  readonly processes: Readonly<Record<ServiceStatusProcessState, string>>;
  readonly work: Readonly<Record<ServiceStatusWorkState, string>>;
  readonly expectedModes: Readonly<Record<ServiceStatusExpectedMode, string>>;
  readonly kinds: Readonly<Record<ServiceStatusKind, string>>;
  readonly names: Readonly<Record<ServiceStatusId, string>>;
  readonly descriptions: Readonly<Record<ServiceStatusId, string>>;
  readonly reasonGroups: Readonly<Record<ServiceStatusReasonGroup, string>>;
  readonly calendarStates: Readonly<
    Record<
      "ready" | "closed" | "missing" | "stale" | "invalid" | "unknown" | "not_applicable",
      string
    >
  >;
  readonly windowStates: Readonly<
    Record<"ready" | "closed" | "missing" | "invalid" | "unknown" | "not_applicable", string>
  >;
};

type ServiceStatusMetricDictionary = {
  last_attempt_at: true;
  last_success_at: true;
  last_failure_at: true;
  last_received_at: true;
  oldest_provider_event_at: true;
  next_run_at: true;
  data_due_at: true;
  data_as_of: true;
  expected_session: true;
  raw_as_of: true;
  v2_data_as_of: true;
  v2_snapshot_as_of: true;
  queued: true;
  running: true;
  recent_failed: true;
  expired_claims: true;
  active_demand: true;
  live_identities: true;
  halted_identities: true;
  required_identities: true;
  covered_identities: true;
  queue_scope: true;
  calendar_status: true;
  window_status: true;
  last_state_change_at: true;
};

export const serviceStatusDictionary: Readonly<Record<Locale, ServiceStatusDictionary>> = {
  en: {
    heading: "Service status",
    disclosureLabel: "Expand or collapse service status details",
    tableCaption: "Service status and recent work evidence",
    serviceHeading: "Service",
    statusHeading: "Overall",
    processHeading: "Process",
    workHeading: "Work and data",
    detailsHeading: "Recent details",
    observedLabel: "Observed",
    observedAt: "Last observed",
    lastRecorded: "Last recorded",
    loading: "Loading service status…",
    unavailable: "Status is unavailable. Previous readings are shown as stale.",
    rateLimited: "Status refresh is being rate limited. Previous readings are stale.",
    denied: "Service details are no longer available for this session.",
    noDetails: "No service details are available.",
    staleEvidence: "Needs a new check",
    policyVerified: "Configuration verified",
    policyUnavailable: "Configuration could not be verified",
    policyMismatch: "Configuration differs from the expected setup",
    policyLabel: "Configuration",
    countSummary: (count) => `${count} services counted`,
    sharedQueue: "Shared work count; shown once",
    noMetrics: "No recent work or data details were reported.",
    metricLabels: {
      last_attempt_at: "Last attempt",
      last_success_at: "Last success",
      last_failure_at: "Last failure",
      last_received_at: "Last quote received",
      oldest_provider_event_at: "Oldest quote event",
      next_run_at: "Next run",
      data_due_at: "Data due",
      data_as_of: "Data through",
      expected_session: "Expected session",
      raw_as_of: "Daily data through",
      v2_data_as_of: "Research data through",
      v2_snapshot_as_of: "Signal set date",
      queued: "Queued",
      running: "In progress",
      recent_failed: "Recent failures",
      expired_claims: "Expired work",
      active_demand: "Active quote requests",
      live_identities: "Live quote instruments",
      halted_identities: "Paused instruments",
      required_identities: "Expected instruments",
      covered_identities: "Covered instruments",
      queue_scope: "Work count",
      calendar_status: "Trading calendar",
      window_status: "Trading hours",
      last_state_change_at: "Last state change",
    },
    statuses: {
      healthy: "Healthy",
      waiting: "Waiting",
      degraded: "Needs attention",
      failed: "Failed",
      unknown: "Unknown",
      disabled: "Disabled",
    },
    processes: {
      running: "Running",
      starting: "Starting",
      inactive: "Not running now",
      failed: "Failed",
      missing: "Missing",
      unknown: "Unknown",
      not_applicable: "Not applicable",
      disabled: "Disabled",
    },
    work: {
      active: "Working",
      idle: "Waiting for work",
      scheduled: "Scheduled",
      closed: "Closed",
      no_demand: "No active work",
      current: "Up to date",
      pending: "Pending",
      stale: "Out of date",
      degraded: "Limited",
      failed: "Failed",
      unknown: "Unknown",
      disabled: "Disabled",
      not_applicable: "Not applicable",
    },
    expectedModes: {
      always: "Expected to stay running",
      daily: "Daily schedule",
      daemon: "Work-driven process",
      capability: "Application feature",
      disabled: "Disabled by policy",
      unknown: "Expectation not verified",
    },
    kinds: {
      capability: "Application feature",
      daemon: "Service process",
      execution: "Execution mode",
      host: "Host service",
      optional: "Optional feature",
      scheduled: "Scheduled task",
    },
    names: {
      postgres: "Database",
      "reverse-proxy": "Web gateway",
      web: "Web application",
      "api-server": "Application API",
      "owner-equity-v2-runner": "Owner research",
      "owner-beta-runner": "Beta calculations",
      "candidate-runner": "Candidate analysis",
      "recommendation-runner": "Recommendations",
      "nt-backtest-worker-1": "Backtest worker 1",
      "nt-backtest-worker-2": "Backtest worker 2",
      "research-worker": "Research executor",
      "kis-eod": "Daily market data",
      "market-stream": "Live quotes",
      "intraday-calendar": "Trading calendar",
      "production-backup": "Daily backup",
      "production-backup-verify": "Backup verification",
      "tls-renewal": "Secure access certificate",
      docker: "Container service",
      tailscaled: "Private network service",
      "paper-scheduler": "Paper trading",
      metrics: "Monitoring",
      "live-node-owner": "Live trading",
    },
    descriptions: {
      postgres: "Stores application data.",
      "reverse-proxy": "Routes web requests to the application.",
      web: "Serves the authenticated web application.",
      "api-server": "Serves authenticated data requests.",
      "owner-equity-v2-runner": "Prepares and publishes Owner research data.",
      "owner-beta-runner": "Processes beta analysis work.",
      "candidate-runner": "Processes candidate analysis work.",
      "recommendation-runner": "Processes recommendation work.",
      "nt-backtest-worker-1": "Processes backtest work.",
      "nt-backtest-worker-2": "Processes backtest work.",
      "research-worker": "Execution mode used by the daily research task.",
      "kis-eod": "Collects and publishes daily market data.",
      "market-stream": "Keeps requested live quotes current.",
      "intraday-calendar": "Confirms trading dates and hours.",
      "production-backup": "Creates the scheduled database backup.",
      "production-backup-verify": "Checks the latest backup.",
      "tls-renewal": "Renews the service certificate.",
      docker: "Runs the application containers.",
      tailscaled: "Maintains private network access.",
      "paper-scheduler": "Optional feature; disabled by policy.",
      metrics: "Optional monitoring; disabled by policy.",
      "live-node-owner": "Optional feature; disabled by policy.",
    },
    reasonGroups: {
      process: "The service process needs attention.",
      schedule: "The scheduled run needs attention.",
      queue: "The recent work needs attention.",
      calendar: "Trading calendar or hours need attention.",
      stream: "Live quote delivery needs attention.",
      data: "Market data needs attention.",
      policy: "The configured service policy needs attention.",
      source: "The status evidence could not be confirmed.",
      expired: "This reading expired and needs a new check.",
      disabled: "This feature is intentionally disabled.",
      information: "No separate work is expected right now.",
    },
    calendarStates: {
      ready: "Ready",
      closed: "Closed",
      missing: "Missing",
      stale: "Out of date",
      invalid: "Needs attention",
      unknown: "Unknown",
      not_applicable: "Not applicable",
    },
    windowStates: {
      ready: "Open",
      closed: "Closed",
      missing: "Missing",
      invalid: "Needs attention",
      unknown: "Unknown",
      not_applicable: "Not applicable",
    },
  },
  ko: {
    heading: "서비스 상태",
    disclosureLabel: "서비스 상태 상세 펼치기 또는 접기",
    tableCaption: "서비스 상태와 최근 작업 근거",
    serviceHeading: "서비스",
    statusHeading: "종합",
    processHeading: "프로세스",
    workHeading: "작업 및 데이터",
    detailsHeading: "최근 상세",
    observedLabel: "관측",
    observedAt: "마지막 확인",
    lastRecorded: "마지막 기록",
    loading: "서비스 상태를 불러오는 중…",
    unavailable: "상태를 확인할 수 없습니다. 이전 기록은 오래된 정보로 표시합니다.",
    rateLimited: "상태 갱신이 제한되었습니다. 이전 기록은 오래된 정보로 표시합니다.",
    denied: "현재 세션에서 서비스 상세를 더 이상 확인할 수 없습니다.",
    noDetails: "표시할 서비스 상세가 없습니다.",
    staleEvidence: "새 확인이 필요합니다",
    policyVerified: "설정 확인 완료",
    policyUnavailable: "설정을 확인할 수 없습니다",
    policyMismatch: "설정이 기대값과 다릅니다",
    policyLabel: "설정",
    countSummary: (count) => `${count}개 서비스 집계`,
    sharedQueue: "공유 작업 수이며 한 번만 표시합니다",
    noMetrics: "최근 작업 또는 데이터 상세가 보고되지 않았습니다.",
    metricLabels: {
      last_attempt_at: "마지막 시도",
      last_success_at: "마지막 성공",
      last_failure_at: "마지막 실패",
      last_received_at: "마지막 시세 수신",
      oldest_provider_event_at: "가장 오래된 시세 이벤트",
      next_run_at: "다음 실행",
      data_due_at: "데이터 기준 시각",
      data_as_of: "데이터 기준일",
      expected_session: "기대 거래일",
      raw_as_of: "일봉 데이터 기준일",
      v2_data_as_of: "리서치 데이터 기준일",
      v2_snapshot_as_of: "신호 묶음 기준일",
      queued: "대기 작업",
      running: "진행 중",
      recent_failed: "최근 실패",
      expired_claims: "만료된 작업",
      active_demand: "활성 시세 요청",
      live_identities: "실시간 시세 종목",
      halted_identities: "일시 중단 종목",
      required_identities: "대상 종목",
      covered_identities: "확인된 종목",
      queue_scope: "작업 수",
      calendar_status: "거래일 달력",
      window_status: "거래 시간",
      last_state_change_at: "마지막 상태 변경",
    },
    statuses: {
      healthy: "정상",
      waiting: "대기",
      degraded: "주의 필요",
      failed: "실패",
      unknown: "확인 불가",
      disabled: "비활성",
    },
    processes: {
      running: "실행 중",
      starting: "시작 중",
      inactive: "현재 미실행",
      failed: "실패",
      missing: "없음",
      unknown: "확인 불가",
      not_applicable: "해당 없음",
      disabled: "비활성",
    },
    work: {
      active: "작업 중",
      idle: "작업 대기",
      scheduled: "예약됨",
      closed: "장 종료",
      no_demand: "활성 작업 없음",
      current: "최신 상태",
      pending: "대기 중",
      stale: "지연됨",
      degraded: "일부 제한",
      failed: "실패",
      unknown: "확인 불가",
      disabled: "비활성",
      not_applicable: "해당 없음",
    },
    expectedModes: {
      always: "상시 실행 대상",
      daily: "일일 예약 작업",
      daemon: "작업 기반 프로세스",
      capability: "애플리케이션 기능",
      disabled: "정책상 비활성",
      unknown: "기대 설정 확인 불가",
    },
    kinds: {
      capability: "애플리케이션 기능",
      daemon: "서비스 프로세스",
      execution: "실행 방식",
      host: "호스트 서비스",
      optional: "선택 기능",
      scheduled: "예약 작업",
    },
    names: {
      postgres: "데이터베이스",
      "reverse-proxy": "웹 게이트웨이",
      web: "웹 애플리케이션",
      "api-server": "애플리케이션 API",
      "owner-equity-v2-runner": "Owner 리서치",
      "owner-beta-runner": "베타 분석",
      "candidate-runner": "후보 분석",
      "recommendation-runner": "추천 작업",
      "nt-backtest-worker-1": "백테스트 작업자 1",
      "nt-backtest-worker-2": "백테스트 작업자 2",
      "research-worker": "리서치 실행기",
      "kis-eod": "일일 시세 데이터",
      "market-stream": "실시간 시세",
      "intraday-calendar": "거래일 달력",
      "production-backup": "일일 백업",
      "production-backup-verify": "백업 검증",
      "tls-renewal": "보안 접속 인증서",
      docker: "컨테이너 서비스",
      tailscaled: "사설 네트워크 서비스",
      "paper-scheduler": "모의 거래",
      metrics: "모니터링",
      "live-node-owner": "실거래 기능",
    },
    descriptions: {
      postgres: "애플리케이션 데이터를 저장합니다.",
      "reverse-proxy": "웹 요청을 애플리케이션으로 전달합니다.",
      web: "인증된 웹 애플리케이션을 제공합니다.",
      "api-server": "인증된 데이터 요청을 처리합니다.",
      "owner-equity-v2-runner": "Owner 리서치 데이터를 준비하고 게시합니다.",
      "owner-beta-runner": "베타 분석 작업을 처리합니다.",
      "candidate-runner": "후보 분석 작업을 처리합니다.",
      "recommendation-runner": "추천 작업을 처리합니다.",
      "nt-backtest-worker-1": "백테스트 작업을 처리합니다.",
      "nt-backtest-worker-2": "백테스트 작업을 처리합니다.",
      "research-worker": "일일 리서치 작업에서 사용하는 실행 방식입니다.",
      "kis-eod": "일일 시세 데이터를 수집하고 게시합니다.",
      "market-stream": "요청된 실시간 시세를 최신 상태로 유지합니다.",
      "intraday-calendar": "거래일과 거래 시간을 확인합니다.",
      "production-backup": "예약된 데이터베이스 백업을 생성합니다.",
      "production-backup-verify": "최근 백업을 확인합니다.",
      "tls-renewal": "서비스 인증서를 갱신합니다.",
      docker: "애플리케이션 컨테이너를 실행합니다.",
      tailscaled: "사설 네트워크 접속을 유지합니다.",
      "paper-scheduler": "선택 기능이며 정책상 비활성입니다.",
      metrics: "선택 모니터링 기능이며 정책상 비활성입니다.",
      "live-node-owner": "선택 기능이며 정책상 비활성입니다.",
    },
    reasonGroups: {
      process: "서비스 프로세스 확인이 필요합니다.",
      schedule: "예약 실행 확인이 필요합니다.",
      queue: "최근 작업 확인이 필요합니다.",
      calendar: "거래일 달력 또는 거래 시간 확인이 필요합니다.",
      stream: "실시간 시세 전달 확인이 필요합니다.",
      data: "시장 데이터 확인이 필요합니다.",
      policy: "서비스 설정 확인이 필요합니다.",
      source: "상태 근거를 확인할 수 없습니다.",
      expired: "관측이 만료되어 새 확인이 필요합니다.",
      disabled: "이 기능은 정책에 따라 의도적으로 비활성화되었습니다.",
      information: "현재 별도 작업이 필요하지 않습니다.",
    },
    calendarStates: {
      ready: "확인 완료",
      closed: "종료",
      missing: "없음",
      stale: "오래됨",
      invalid: "확인 필요",
      unknown: "확인 불가",
      not_applicable: "해당 없음",
    },
    windowStates: {
      ready: "운영 중",
      closed: "종료",
      missing: "없음",
      invalid: "확인 필요",
      unknown: "확인 불가",
      not_applicable: "해당 없음",
    },
  },
};
