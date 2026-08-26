import type { LocaleDictionary } from "@/lib/i18n/locale";

export type StrategiesDictionary = {
  readonly allowedMaximumFallback: string;
  readonly allowedMinimumFallback: string;
  readonly allowedParametersLegend: string;
  readonly blockedCatalogMessage: string;
  readonly blockedCatalogTitle: string;
  readonly catalogEmptyMessage: string;
  readonly catalogEmptyTitle: string;
  readonly catalogEyebrow: string;
  readonly catalogHeading: string;
  readonly configurationSaved: (id: string) => string;
  readonly configurationUnavailableMessage: string;
  readonly configureAriaLabel: (displayName: string) => string;
  readonly fieldMustBeBetween: (title: string, minimum: string, maximum: string) => string;
  readonly fieldMustBeGreaterThan: (title: string, minimum: string) => string;
  readonly fieldMustMatchPattern: (title: string) => string;
  readonly fieldMustBeOneOf: (title: string) => string;
  readonly fieldMustBeValidType: (title: string, type: string) => string;
  readonly fieldRequired: (title: string) => string;
  readonly noRiskDescriptionReported: string;
  readonly noSchemaAvailable: string;
  readonly noStrategyDescriptionReported: string;
  readonly notReported: string;
  readonly retiredMessage: string;
  readonly riskWarningLabel: string;
  readonly routeDescription: string;
  readonly routeTitle: string;
  readonly saveStrategyConfiguration: string;
  readonly savingConfiguration: string;
  readonly schemaBoundNote: string;
  readonly unavailableCatalogMessage: string;
  readonly unavailableCatalogTitle: string;
  readonly versionLabel: (version: string) => string;
};

export const strategiesDictionary: LocaleDictionary<StrategiesDictionary> = {
  en: {
    allowedMaximumFallback: "the allowed maximum",
    allowedMinimumFallback: "the allowed minimum",
    allowedParametersLegend: "Allowed parameters",
    blockedCatalogMessage:
      "Strategy configuration is blocked because the required data entitlement is inactive. No configuration was submitted.",
    blockedCatalogTitle: "Strategy configuration is blocked",
    catalogEmptyMessage:
      "The server returned no approved strategies. No configuration can be created.",
    catalogEmptyTitle: "Strategy catalog is empty",
    catalogEyebrow: "Approved catalog",
    catalogHeading: "Strategy versions",
    configurationSaved: (id) => `Configuration saved (${id}).`,
    configurationUnavailableMessage:
      "Configuration is unavailable while the required data entitlement is inactive.",
    configureAriaLabel: (displayName) => `Configure ${displayName}`,
    fieldMustBeBetween: (title, minimum, maximum) =>
      `${title} must be between ${minimum} and ${maximum}.`,
    fieldMustBeGreaterThan: (title, minimum) => `${title} must be greater than ${minimum}.`,
    fieldMustMatchPattern: (title) => `${title} has an invalid format.`,
    fieldMustBeOneOf: (title) => `${title} must be one of the allowed values.`,
    fieldMustBeValidType: (title, type) => `${title} must be a valid ${type}.`,
    fieldRequired: (title) => `${title} is required.`,
    noRiskDescriptionReported: "No additional risk description was reported.",
    noSchemaAvailable: "No configurable parameter schema is available.",
    noStrategyDescriptionReported: "No strategy description was reported.",
    notReported: "Not reported",
    retiredMessage: "This strategy version is retired and cannot be configured.",
    riskWarningLabel: "Risk warning",
    routeDescription:
      "Review approved strategy definitions, versions, states, and constrained parameters.",
    routeTitle: "Strategies",
    saveStrategyConfiguration: "Save strategy configuration",
    savingConfiguration: "Saving configuration",
    schemaBoundNote:
      "Only schema-bound parameters can be changed. Strategy code remains server-managed.",
    unavailableCatalogMessage:
      "The strategy catalog could not be loaded. Retry after checking the service status.",
    unavailableCatalogTitle: "Strategy catalog unavailable",
    versionLabel: (version) => `Version ${version}`,
  },
  ko: {
    allowedMaximumFallback: "허용 최댓값",
    allowedMinimumFallback: "허용 최솟값",
    allowedParametersLegend: "허용된 파라미터",
    blockedCatalogMessage:
      "필요한 데이터 이용 권한이 비활성 상태이므로 전략 설정이 차단되었습니다. 설정이 제출되지 않았습니다.",
    blockedCatalogTitle: "전략 설정이 차단되었습니다",
    catalogEmptyMessage: "서버가 승인된 전략을 반환하지 않았습니다. 설정을 생성할 수 없습니다.",
    catalogEmptyTitle: "전략 카탈로그가 비어 있습니다",
    catalogEyebrow: "승인된 카탈로그",
    catalogHeading: "전략 버전",
    configurationSaved: (id) => `설정이 저장되었습니다 (${id}).`,
    configurationUnavailableMessage:
      "필요한 데이터 이용 권한이 비활성 상태이므로 설정을 이용할 수 없습니다.",
    configureAriaLabel: (displayName) => `${displayName} 설정`,
    fieldMustBeBetween: (title, minimum, maximum) =>
      `${title}은(는) ${minimum}에서 ${maximum} 사이여야 합니다.`,
    fieldMustBeGreaterThan: (title, minimum) => `${title}은(는) ${minimum}보다 커야 합니다.`,
    fieldMustMatchPattern: (title) => `${title}의 형식이 올바르지 않습니다.`,
    fieldMustBeOneOf: (title) => `${title}은(는) 허용된 값 중 하나여야 합니다.`,
    fieldMustBeValidType: (title, type) => `${title}은(는) 유효한 ${type} 값이어야 합니다.`,
    fieldRequired: (title) => `${title}은(는) 필수입니다.`,
    noRiskDescriptionReported: "추가 리스크 설명이 보고되지 않았습니다.",
    noSchemaAvailable: "이용 가능한 파라미터 스키마가 없습니다.",
    noStrategyDescriptionReported: "전략 설명이 보고되지 않았습니다.",
    notReported: "보고되지 않음",
    retiredMessage: "이 전략 버전은 폐기되어 설정할 수 없습니다.",
    riskWarningLabel: "리스크 경고",
    routeDescription: "승인된 전략 정의, 버전, 상태, 제한된 파라미터를 확인하세요.",
    routeTitle: "전략",
    saveStrategyConfiguration: "전략 설정 저장",
    savingConfiguration: "설정 저장 중",
    schemaBoundNote:
      "스키마에 정의된 파라미터만 변경할 수 있습니다. 전략 코드는 서버에서 관리됩니다.",
    unavailableCatalogMessage:
      "전략 카탈로그를 불러오지 못했습니다. 서비스 상태를 확인한 후 다시 시도하세요.",
    unavailableCatalogTitle: "전략 카탈로그를 이용할 수 없습니다",
    versionLabel: (version) => `버전 ${version}`,
  },
};
