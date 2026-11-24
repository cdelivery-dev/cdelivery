# 用 AI 安装并配置 CDelivery

先按[通用 AI 安装指南](https://cdelivery.dev/install.md)第 1、2 节确认目标服务器，检查是否已安装 CDelivery。已有安装沿用当前镜像源，按通用指南检查服务并完成所需配置。首次安装按以下步骤操作。

## 安装准备

安装前，请在服务器上运行以下命令。

```sh
(
  set -eu
  probe_dir=$(mktemp -d "${TMPDIR:-/tmp}/cdelivery-probe.XXXXXX")
  trap 'rm -rf "$probe_dir"' 0
  cd "$probe_dir"
  curl -qfsSL --proto '=https' --proto-redir '=https' \
    --connect-timeout 15 --max-time 120 \
    https://cdelivery.dev/install-cn.sh -o install-cn.sh
  sh install-cn.sh --probe-only
)
```

用简体中文列出两种安装方式，标明推荐项。若两项都不可用，请用户选择重试或指定镜像源。

## 按用户选择安装

等用户确认安装方式后，在通用指南第 3 节的 `install.sh` 命令中加入对应参数，保留为目标服务器选定的通信地址。

| 用户选择 | 安装参数 |
| --- | --- |
| DaoCloud 加速 | `--image-prefix m.daocloud.io/ghcr.io/cdelivery-dev` |
| GHCR 直连 | `--image-prefix ghcr.io/cdelivery-dev` |

按通用指南验证安装结果，再完成管理员设置、平台配置和 AI 工具连接。
