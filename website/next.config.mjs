import { createMDX } from "fumadocs-mdx/next";

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  async redirects() {
    return [
      {
        source: "/docs/reference",
        destination: "/reference",
        permanent: true,
      },
      {
        source: "/docs/reference/:path*",
        destination: "/reference/:path*",
        permanent: true,
      },
    ];
  },
};

export default withMDX(config);
