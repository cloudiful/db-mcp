FROM nexus.cloud1ful.com/dockerhub/library/golang:1.25.5-alpine AS builder

WORKDIR /src

COPY go.mod go.sum ./
RUN go mod download

COPY . .

RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath -ldflags="-s -w" -o /out/dbmcp ./cmd/dbmcp

FROM nexus.cloud1ful.com/dockerhub/library/alpine:3.22

RUN addgroup -S app && adduser -S -G app app \
    && apk add --no-cache ca-certificates

WORKDIR /app

COPY --from=builder /out/dbmcp /app/dbmcp

USER app

EXPOSE 8080

ENTRYPOINT ["/app/dbmcp"]
CMD ["-transport", "http", "-http-addr", ":8080"]
