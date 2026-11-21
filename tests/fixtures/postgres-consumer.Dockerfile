FROM busybox:1.37.0-musl AS busybox
FROM postgres:18-alpine
COPY --from=busybox /bin/busybox /opt/busybox
COPY postgres-health.sh /www/cgi-bin/health
RUN chmod 755 /www/cgi-bin/health
ENTRYPOINT []
CMD ["/opt/busybox", "httpd", "-f", "-p", "8080", "-h", "/www"]
