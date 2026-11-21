#!/bin/sh
set -eu
export PGCONNECT_TIMEOUT=5
if result=$(psql "$DATABASE_URL" -XAt -v ON_ERROR_STOP=1 -c 'SELECT value FROM fixture WHERE id = 1' 2>/dev/null) &&
    [ "$result" = healthy ] &&
    result=$(PGPASSWORD="$DB_PASSWORD" psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USERNAME" -d "$DB_DATABASE" -XAt -v ON_ERROR_STOP=1 -c 'SELECT value FROM fixture WHERE id = 1' 2>/dev/null) &&
    [ "$result" = healthy ]; then
    printf 'Content-Type: text/plain\r\n\r\nhealthy'
else
    printf 'Status: 503 Service Unavailable\r\nContent-Type: text/plain\r\n\r\ndatabase check failed'
fi
