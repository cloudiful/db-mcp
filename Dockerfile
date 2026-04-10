FROM dockerhub.cloud1ful.com/library/golang:1.26.2-trixie AS builder

WORKDIR /src

COPY go.mod go.sum ./
RUN go mod download

COPY . .

RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath -ldflags="-s -w" -o /out/dbmcp ./cmd/dbmcp

FROM dockerhub.cloud1ful.com/library/debian:trixie-slim

RUN rm -f /etc/apt/sources.list \
    && printf '%s\n' \
        'Types: deb' \
        'URIs: https://apt.cloud1ful.com/deb.debian.org/debian' \
        'Suites: trixie trixie-updates' \
        'Components: main contrib non-free non-free-firmware' \
        'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
        '' \
        'Types: deb' \
        'URIs: https://apt.cloud1ful.com/deb.debian.org/debian-security' \
        'Suites: trixie-security' \
        'Components: main contrib non-free non-free-firmware' \
        'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
        > /etc/apt/sources.list.d/debian.sources \
    && printf '%s\n' \
        'Acquire::https::apt.cloud1ful.com::Verify-Peer "false";' \
        'Acquire::https::apt.cloud1ful.com::Verify-Host "false";' \
        > /etc/apt/apt.conf.d/99apt-cloud1ful-insecure \
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
