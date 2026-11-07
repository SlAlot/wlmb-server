#!/bin/sh
# 分支软件源会替换旧包；镜像仅作下载回退，所有来源都必须匹配固定摘要。
set -eu
output="${1:?需要输出文件}"
name='apk-tools-static-3.0.9-r0.apk'
digest='ea77b2533566e70c9566c03748199b79348010c3199ba02d97e0634cf0fa9ef7'
for base in \
  'https://dl-cdn.alpinelinux.org/alpine/v3.23/main/x86_64' \
  'https://mirrors.edge.kernel.org/alpine/v3.23/main/x86_64' \
  'https://mirrors.huaweicloud.com/repository/alpine/v3.23/main/x86_64'
do
  if curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
      --max-time 40 --retry 2 --output "$output" "$base/$name" &&
      printf '%s  %s\n' "$digest" "$output" | sha256sum --check --status
  then
    exit 0
  fi
done
printf '%s\n' 'APK 工具未找到或摘要不匹配；停止构建。' >&2
exit 1
