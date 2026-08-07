const DECIMAL_PATTERN = /^(?<sign>-?)(?<integer>\d+)(?:\.(?<fraction>\d+))?$/;

export class DecimalFormatError extends Error {
  override readonly name = "DecimalFormatError";
}

type ParsedDecimal = {
  readonly negative: boolean;
  readonly scale: number;
  readonly unscaled: bigint;
};

function parseDecimal(value: string): ParsedDecimal {
  const match = DECIMAL_PATTERN.exec(value);
  if (match?.groups === undefined) {
    throw new DecimalFormatError(`Invalid decimal value: ${value}`);
  }
  const fraction = match.groups["fraction"] ?? "";
  return {
    negative: match.groups["sign"] === "-",
    scale: fraction.length,
    unscaled: BigInt(`${match.groups["integer"]}${fraction}`),
  };
}

function roundHalfEven(value: bigint, divisor: bigint): bigint {
  const quotient = value / divisor;
  const remainder = value % divisor;
  const doubled = remainder * 2n;
  if (doubled > divisor || (doubled === divisor && quotient % 2n !== 0n)) {
    return quotient + 1n;
  }
  return quotient;
}

function scaledValue(parsed: ParsedDecimal, fractionDigits: number): bigint {
  if (parsed.scale === fractionDigits) {
    return parsed.unscaled;
  }
  if (parsed.scale < fractionDigits) {
    return parsed.unscaled * 10n ** BigInt(fractionDigits - parsed.scale);
  }
  return roundHalfEven(parsed.unscaled, 10n ** BigInt(parsed.scale - fractionDigits));
}

function grouped(integer: string): string {
  return integer.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

export function formatDecimal(
  value: string,
  fractionDigits = 2,
  useGrouping = false,
): string {
  if (!Number.isInteger(fractionDigits) || fractionDigits < 0 || fractionDigits > 12) {
    throw new DecimalFormatError("fractionDigits must be an integer between 0 and 12");
  }
  const parsed = parseDecimal(value);
  const digits = scaledValue(parsed, fractionDigits).toString().padStart(fractionDigits + 1, "0");
  const integer = fractionDigits === 0 ? digits : digits.slice(0, -fractionDigits);
  const fraction = fractionDigits === 0 ? "" : `.${digits.slice(-fractionDigits)}`;
  const sign = parsed.negative && parsed.unscaled !== 0n ? "−" : "";
  return `${sign}${useGrouping ? grouped(integer) : integer}${fraction}`;
}

export function formatPercentage(value: string, fractionDigits = 2): string {
  const parsed = parseDecimal(value);
  const percentage = {
    ...parsed,
    unscaled: parsed.unscaled * 100n,
  };
  const normalized = `${percentage.negative ? "-" : ""}${percentage.unscaled.toString()}`;
  const scale = percentage.scale;
  const decimal =
    scale === 0
      ? normalized
      : `${percentage.negative ? "-" : ""}${percentage.unscaled
          .toString()
          .padStart(scale + 1, "0")
          .slice(0, -scale)}.${percentage.unscaled.toString().padStart(scale + 1, "0").slice(-scale)}`;
  return `${formatDecimal(decimal, fractionDigits)}%`;
}

export function formatKrw(value: string): string {
  return `₩${formatDecimal(value, 2, true)}`;
}

const DATE_FORMATTER = new Intl.DateTimeFormat("en-US", {
  day: "numeric",
  month: "short",
  timeZone: "UTC",
  year: "numeric",
});

const TIMESTAMP_FORMATTER = new Intl.DateTimeFormat("en-US", {
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  month: "short",
  timeZone: "Asia/Seoul",
  year: "numeric",
});

export function formatDate(value: string): string {
  return DATE_FORMATTER.format(new Date(`${value}T00:00:00Z`));
}

export function formatTimestamp(value: string): string {
  return `${TIMESTAMP_FORMATTER.format(new Date(value))} KST`;
}
