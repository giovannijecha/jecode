#!/bin/sh
# The Linux runner's outer sandbox disables nested user namespaces.
exec /opt/google/chrome/chrome --no-sandbox "$@"
