#!/bin/bash

dnf update -y
dnf install -y nginx

cat > /usr/share/nginx/html/index.html <<EOF
<!DOCTYPE html>
<html>
  <head>
    <meta charset="UTF-8">
    <title>Szeretlek Timi ❤️</title>
  </head>
  <body>
    <h1>Szeretlek Timi ❤️</h1>
  </body>
</html>
EOF

systemctl enable nginx
systemctl start nginx
