#!/bin/sh
set -e

PASSWORD="test-password"
POSTGRES_VERSION=16

docker run --rm -e POSTGRES_PASSWORD="$PASSWORD" --publish 5432:5432 --detach --tty postgres:"$POSTGRES_VERSION"
sleep 2 # wait for service to be up
docker run --rm --network host --tty postgres:"$POSTGRES_VERSION" psql postgres://postgres:test-password@localhost -c "CREATE DATABASE todo;"
