import type { NextConfig } from 'next';

// A static export, served by Pratibimba Core at /app (one origin, one deploy).
const config: NextConfig = {
  output: 'export',
  basePath: '/app',
  trailingSlash: true,
  images: { unoptimized: true },
};
export default config;
