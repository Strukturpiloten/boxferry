#!/usr/bin/env python3
"""Require successful immediate revocation; never log credentials or API payloads."""

from __future__ import annotations

import os
import sys
import urllib.request


ENDPOINT = "https://api.github.com/installation/token"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def revoke(token, *, opener=None):
    if type(token) is not str or not token or any(character in token for character in "\r\n"):
        raise ValueError("rejected")
    request = urllib.request.Request(ENDPOINT, method="DELETE", headers={
        "Authorization": "Bearer " + token, "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "boxferry-reviewed-admission",
    })
    # Disable redirects and ambient proxies; the credential has one fixed destination.
    opener = opener or urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=10) as response:
        if response.status != 204 or response.geturl() != ENDPOINT:
            raise ValueError("rejected")


def main():
    try:
        if len(sys.argv) != 1:
            raise ValueError("rejected")
        revoke(os.environ.get("DOCKERLENS_ADMISSION_TOKEN", ""))
    except Exception:
        print("reviewed Docker admission token revocation rejected", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
