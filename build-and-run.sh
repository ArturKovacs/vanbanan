#!/bin/bash

set -eu

./build.sh

cd ./dist
./liba-back
