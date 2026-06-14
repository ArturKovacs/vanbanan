#!/bin/bash

# The first argument to this script is the hostname
TARGET_HOST=$1

if [ -z "$TARGET_HOST" ]; then
    echo "Usage: $0 <target_host>"
    echo "Example: $0 ec2-user@10.0.0.1"
    exit 1
fi

echo "Uploading dist contents to $TARGET_HOST:/home/ec2-user/LIBA/"
ssh "$TARGET_HOST" "mkdir -p /home/ec2-user/LIBA"
echo "Stopping liba-back service..."
ssh "$TARGET_HOST" "sudo systemctl stop liba-back || true"
ssh "$TARGET_HOST" "until ! sudo systemctl is-active --quiet liba-back; do sleep 0.5; done"

# move the database files just until the copying finishes
mkdir -p "./tmpdb/"
mv ./dist/*db.msgpack ./tmpdb/
scp -r ./dist/* "$TARGET_HOST":/home/ec2-user/LIBA/
mv ./tmpdb/*db.msgpack ./dist/ 

ssh "$TARGET_HOST" "sudo chown -R ec2-user:ec2-user /home/ec2-user/LIBA"
ssh "$TARGET_HOST" "sudo chmod +x /home/ec2-user/LIBA/liba-back"

ssh "$TARGET_HOST" "sudo systemctl enable --now liba-back"
echo "$(date): liba-back service started"
