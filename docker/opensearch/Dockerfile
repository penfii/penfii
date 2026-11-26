# OpenSearch with the Korean analyzer (nori) that the search mapping needs.
FROM opensearchproject/opensearch:3.10.0
RUN /usr/share/opensearch/bin/opensearch-plugin install --batch analysis-nori
# Penfii uses none of these bundled plugins (it needs indexing, search and nori), and they carry the vulnerable
# libraries (bcprov, jline, spark/jetty) that the base image's scan reports: the performance analyzer, ML Commons
# and the skills plugin. The rest of the bundle stays as upstream ships it (the security plugin is switched off
# by the Compose file's DISABLE_SECURITY_PLUGIN; OpenSearch is only reachable on the internal `data` network).
RUN for p in opensearch-performance-analyzer opensearch-skills opensearch-ml; do /usr/share/opensearch/bin/opensearch-plugin remove "$p"; done
