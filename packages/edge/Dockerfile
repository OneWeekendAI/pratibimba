# The edge as a sidecar: sits in front of your origin and rewrites marked slots as the HTML
# streams. Build context is this directory:
#   docker build -t pratibimba-edge packages/edge
#   docker run -e PRB_CORE_URL=http://core:8080 -e PRB_SITE=shop -e PRB_ORIGIN_URL=http://app:3000 \
#     -p 8787:8787 pratibimba-edge
FROM node:24-trixie-slim AS build
WORKDIR /edge
COPY package.json package-lock.json tsconfig.json ./
RUN npm ci --no-audit --no-fund --ignore-scripts
COPY src src
RUN npx tsc -p tsconfig.json

FROM node:24-trixie-slim
ENV NODE_ENV=production PRB_EDGE_PORT=8787
WORKDIR /app
# No runtime dependencies: dist is plain ESM on Node's own http and fetch.
COPY --from=build /edge/package.json ./
COPY --from=build /edge/dist dist
USER node
EXPOSE 8787
HEALTHCHECK --interval=10s --timeout=3s --start-period=3s --retries=3 \
  CMD node -e "fetch('http://127.0.0.1:'+process.env.PRB_EDGE_PORT+'/__prb_edge/health').then(r=>process.exit(r.ok?0:1),()=>process.exit(1))"
ENTRYPOINT ["node", "dist/cli.js"]
