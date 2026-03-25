# 使用最精简的 Node.js 镜像
FROM node:lts-trixie-slim

RUN apt-get update && \
    apt-get install -y git --no-install-recommends && \
    rm -rf /var/lib/apt/lists/*

ENV ONNXRUNTIME_NODE_INSTALL_CUDA="skip"

# 直接安装 gitnexus
RUN --mount=type=cache,target=/root/.npm \
    npm install -g gitnexus --loglevel=verbose

# 设置工作目录
WORKDIR /repos

# 暴露 API 端口（请确认 gitnexus 的默认端口，通常为 3000 或 8000）
EXPOSE 4747

ENV SERVE_PORT=4747

# 启动服务
# 注意：启动参数需要根据 gitnexus 的 CLI 文档调整
# 假设 gitnexus serve 可以通过参数指定 registry 路径
ENTRYPOINT ["sh", "-c", "gitnexus serve -p ${SERVE_PORT} --host 0.0.0.0"]
