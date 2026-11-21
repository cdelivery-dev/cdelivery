#!/bin/sh
set -eu

if [ "$#" -eq 0 ]; then
    set -- --load
fi

exec docker buildx build --tag ghcr.io/cdelivery-dev/hello:1.0.0 "$@" - <<'DOCKERFILE'
FROM busybox:musl

COPY --chmod=755 <<'CGI' /www/cgi-bin/index.cgi
#!/bin/sh
printf 'Content-Type: text/plain; charset=utf-8\r\n\r\n'
printf 'Hostname: %s\n' "$(hostname)"
printf 'IP: %s\n' "$(hostname -i)"
printf 'RemoteAddr: %s\n' "$REMOTE_ADDR"
printf 'Method: %s\n' "$REQUEST_METHOD"
printf 'URI: %s\n' "$REQUEST_URI"
printf 'Host: %s\n' "$HTTP_HOST"
printf 'User-Agent: %s\n' "$HTTP_USER_AGENT"
printf 'X-Forwarded-For: %s\n' "$HTTP_X_FORWARDED_FOR"
printf 'X-Forwarded-Proto: %s\n' "$HTTP_X_FORWARDED_PROTO"
CGI

EXPOSE 80
CMD ["httpd", "-f", "-v", "-p", "80", "-h", "/www"]
DOCKERFILE
