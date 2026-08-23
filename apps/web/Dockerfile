# syntax=docker/dockerfile:1.7

# Keep this image in lockstep with the runtime image selected by
# deploy/compose/compose.yml. Build from the repository root:
#   docker build -f apps/web/Dockerfile .
# Install from the workspace lockfile. Only package manifests are copied into
# this stage, so source files and any local environment files cannot affect the
# dependency layer.
FROM node@sha256:4f696fbf39f383c1e486030ba6b289a5d9af541642fc78ab197e584a113b9c03 AS deps
ENV NEXT_TELEMETRY_DISABLED=1
WORKDIR /app
COPY package.json package-lock.json ./
COPY apps/api-server/package.json apps/api-server/package.json
COPY apps/web/package.json apps/web/package.json
RUN npm ci --ignore-scripts

# Build the web workspace with the generated API contract available for the
# TypeScript path alias. Explicit source copies prevent local secrets and
# unrelated workspace files from entering the image.
FROM node@sha256:4f696fbf39f383c1e486030ba6b289a5d9af541642fc78ab197e584a113b9c03 AS builder
ENV NEXT_TELEMETRY_DISABLED=1
ARG API_INTERNAL_URL=http://api-server:8080
ENV API_INTERNAL_URL=${API_INTERNAL_URL}
WORKDIR /app
COPY --from=deps /app/node_modules ./node_modules
COPY --from=deps /app/package.json /app/package-lock.json ./
COPY apps/api-server/generated/openapi.ts apps/api-server/generated/openapi.ts
COPY apps/web/app apps/web/app
COPY apps/web/components apps/web/components
COPY apps/web/lib apps/web/lib
COPY apps/web/biome.json apps/web/next.config.ts apps/web/package.json apps/web/tsconfig.json ./apps/web/
WORKDIR /app/apps/web
RUN npm run build

# The standalone output contains the production server and its traced runtime
# dependencies only. It is run as a dedicated UID and never needs to write to
# the application tree, which keeps it compatible with a read-only rootfs
# (Compose provides a tmpfs for /tmp).
FROM node@sha256:4f696fbf39f383c1e486030ba6b289a5d9af541642fc78ab197e584a113b9c03 AS runner
ARG LAGRANGE_CODE_COMMIT
RUN test -n "$LAGRANGE_CODE_COMMIT" \
    && printf '%s' "$LAGRANGE_CODE_COMMIT" | grep -Eq '^[0-9a-f]{40}$'
LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    HOSTNAME=0.0.0.0 \
    PORT=3000

RUN addgroup -S -g 10001 nextjs \
    && adduser -S -D -H -u 10001 -G nextjs nextjs
WORKDIR /app

COPY --from=builder --chown=nextjs:nextjs /app/apps/web/.next/standalone ./
COPY --from=builder --chown=nextjs:nextjs /app/apps/web/.next/static ./apps/web/.next/static

USER 10001:10001
EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=5s --start-period=15s --retries=3 \
    CMD node -e "fetch('http://127.0.0.1:3000/healthz').then((response) => { if (!response.ok) process.exit(1); }).catch(() => process.exit(1))"
CMD ["node", "apps/web/server.js"]
