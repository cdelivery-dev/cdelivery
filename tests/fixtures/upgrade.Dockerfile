ARG BASE
FROM ${BASE} AS upgrade
ARG VERSION
LABEL dev.cdelivery.tests.version=${VERSION}

FROM upgrade AS broken
ENTRYPOINT ["/cdelivery-acceptance-missing-entrypoint"]
