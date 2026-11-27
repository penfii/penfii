# OpenSearch with the Korean analyzer (nori) that the search mapping needs.
FROM opensearchproject/opensearch:2.19.8
RUN /usr/share/opensearch/bin/opensearch-plugin install --batch analysis-nori
# Penfii uses none of these bundled plugins (it needs indexing, search and nori through the core REST APIs), and they
# carry the vulnerable libraries that the base image's scan reports: bcprov, jline, spark/jetty (performance analyzer,
# ML Commons, skills); guava, snappy-java and lz4-java (the security plugin; guava again in job scheduler); json (SQL,
# observability, reports scheduler, neural search). Removed with the official `opensearch-plugin remove`, in an order
# that keeps every plugin's dependencies (job scheduler goes only together with everything that extends it: anomaly
# detection, geospatial, index management, reports scheduler, security analytics, SQL). The rest of the bundle stays
# as upstream ships it.
# Without the security plugin the node has no authentication at all: OpenSearch must stay reachable on the internal
# Compose `data` network only (the Compose file does not publish it). The base image's entrypoint skips its
# DISABLE_SECURITY_PLUGIN / demo-configuration step when the plugin is absent, so those Compose settings are inert now.
# `opensearch-plugin remove` leaves a plugin's config directory behind (for the security plugin: its default users and
# roles files), so those go too. Nothing under data/ is touched.
RUN set -eu; \
    cd /usr/share/opensearch; \
    for p in opensearch-performance-analyzer opensearch-skills opensearch-ml \
             opensearch-security-analytics opensearch-anomaly-detection opensearch-geospatial \
             opensearch-index-management opensearch-reports-scheduler opensearch-sql opensearch-job-scheduler \
             opensearch-security opensearch-observability opensearch-neural-search; do \
      bin/opensearch-plugin remove "$p"; \
      rm -rf "config/$p"; \
    done
