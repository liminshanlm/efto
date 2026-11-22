#!/usr/bin/env bash
# 创建/复用本地开发签名身份（自签证书 "efto-dev"），解决 TCC 授权随重编译失效：
# adhoc 签名每次编译 CDHash 都变，系统当成新应用；换成稳定证书身份后，
# 屏幕录制/辅助功能的授权对后续重编译持续有效。
#
#   scripts/sign-dev.sh          # 幂等：已有证书就跳过创建
#
# 证书进 login 钥匙串，可随时删除：
#   security delete-certificate -c efto-dev
# 首次 codesign 可能弹钥匙串授权框，点"始终允许"即可。
set -euo pipefail

CN="${EFTO_SIGN_IDENTITY:-efto-dev}"

if security find-identity -v -p codesigning 2>/dev/null | grep -q "$CN"; then
  echo "==> 签名身份已存在: $CN"
  exit 0
fi

echo "==> 创建自签代码签名证书: ${CN}（进 login 钥匙串）"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# 自签证书：codeSigning EKU，10 年有效
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$tmp/key.pem" -out "$tmp/cert.pem" -days 3650 \
  -subj "/CN=$CN" \
  -addext "keyUsage=digitalSignature" \
  -addext "extendedKeyUsage=codeSigning" 2>/dev/null

# 打包 p12（空密码）导入 login 钥匙串；-T 允许 codesign 使用
# OpenSSL 3 的新默认 MAC/加密 security 工具读不动（MAC verification failed），
# 必须指定 PBE-SHA1-3DES 兼容参数
openssl pkcs12 -export -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
  -out "$tmp/cert.p12" -passout pass:efto-dev -name "$CN" \
  -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 2>/dev/null
security import "$tmp/cert.p12" -k ~/Library/Keychains/login.keychain-db \
  -T /usr/bin/codesign -P efto-dev -f pkcs12

# 标记为受信任的代码签名根（会弹一次系统授权框）
if security add-trusted-cert -d -r trustRoot -p codeSign \
    -k ~/Library/Keychains/login.keychain-db "$tmp/cert.pem" 2>/dev/null; then
  echo "==> 已标记为受信任的代码签名证书"
else
  echo "!! 无法自动设置信任（可能需要在弹出的系统框里确认，或稍后手动信任）"
  echo "!! 证书已导入，codesign 仍可使用；若签名报不受信任，双击证书手动设为始终信任"
fi

if security find-identity -v -p codesigning 2>/dev/null | grep -q "$CN"; then
  echo "==> 完成: $CN"
else
  echo "!! 未在 find-identity 中看到 $CN，检查导入输出" >&2
  exit 1
fi
