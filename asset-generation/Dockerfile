# syntax=docker/dockerfile:1

FROM ubuntu:24.04

ENV VIRTUAL_ENV=/opt/sfx-venv
ENV PATH="$VIRTUAL_ENV/bin:$PATH"
ENV SFX_MODEL_ID=stabilityai/stable-audio-open-1.0

COPY --from=ghcr.io/astral-sh/uv:0.11.24 /uv /usr/local/bin/uv

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
       ca-certificates imagemagick libgomp1 libsndfile1 make povray povray-includes \
       python3 \
    && rm -rf /var/lib/apt/lists/* \
    && uv venv "$VIRTUAL_ENV" --python python3
