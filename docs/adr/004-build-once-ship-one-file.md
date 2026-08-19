# ADR-004：生产环境不运行时安装依赖

状态：Accepted

## 决策

所有依赖必须在构建期锁定并打包。生产部署不要求 Node、npm、pnpm、`node_modules` 或 TypeScript Compiler。

## 产物

`tysel build` 输出单个原生可执行文件。
