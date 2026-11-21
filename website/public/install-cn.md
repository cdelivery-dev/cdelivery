# 用 AI 安装并配置 CDelivery

先按[通用 AI 安装指南](https://cdelivery.dev/install.md)第 1、2 节确认目标服务器，检查是否已安装 CDelivery。已有安装沿用当前镜像源，按通用指南检查服务并完成所需配置。首次安装按以下步骤选择镜像源。

## 在目标服务器上测速

在准备安装 CDelivery 的服务器上运行以下命令。脚本会分别从 DaoCloud（加速安装）和 GHCR（官方直连）下载 512 KiB 样本，输出结果后退出。

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

用简体中文列出两个源各自的耗时或错误，并告诉用户脚本推荐了哪个源。若两个源都失败，展示两项错误，让用户决定重新测速或指定镜像源。

## 按用户选择安装

等用户确认镜像源后，在通用指南第 3 节的 `install.sh` 命令中加入对应参数，保留为目标服务器选定的通信地址。

| 用户选择 | 安装参数 |
| --- | --- |
| 加速安装（DaoCloud） | `--image-prefix m.daocloud.io/ghcr.io/cdelivery-dev` |
| 官方直连（GHCR） | `--image-prefix ghcr.io/cdelivery-dev` |

按通用指南验证安装结果，再完成管理员设置、平台配置和 AI 工具连接。
