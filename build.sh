#!/bin/bash

set -eu

project_root=$(pwd)
dist_folder="$project_root/dist"
dist_static_folder="$project_root/dist/static"
mkdir -p "$dist_static_folder"

cd "$project_root/front"
elm make src/Main.elm --output="$dist_static_folder/main.js"
cp static/* "$dist_static_folder"

cd "$project_root/back"
cargo build --release
cp target/release/liba-back "$dist_folder"
cp vapid_private_key.pem "$dist_folder"
cp vapid_public_key.txt "$dist_folder"
