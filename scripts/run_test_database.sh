#!/bin/sh
set -e

PASSWORD="test-password"

docker run --rm -e POSTGRES_PASSWORD="$PASSWORD" --publish 5432:5432 --detach --tty postgres
sleep 2 # wait for service to be up
docker run --rm --network host --detach --tty postgres psql postgres://postgres:test-password@localhost -c "CREATE DATABASE todo;"
