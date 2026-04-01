FROM nexus.cloud1ful.com/dockerhub/library/golang:1.25-trixie AS builder

WORKDIR /src

COPY go.mod go.sum ./
RUN go mod download

COPY . .

RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath -ldflags="-s -w" -o /out/dbmcp ./cmd/dbmcp

FROM nexus.cloud1ful.com/dockerhub/library/debian:13-slim

RUN printf '%s\n' \
        'deb https://nexus.cloud1ful.com/repository/apt-debian trixie main' \
        'deb https://nexus.cloud1ful.com/repository/apt-debian trixie-updates main' \
        'deb https://nexus.cloud1ful.com/repository/apt-debian-security trixie-security main' \
        > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system app \
    && useradd --system --gid app --home-dir /app --shell /usr/sbin/nologin app

WORKDIR /app

COPY --from=builder /out/dbmcp /app/dbmcp

USER app

EXPOSE 8080

ENTRYPOINT ["/app/dbmcp"]
CMD ["-transport", "http", "-http-addr", ":8080"]
