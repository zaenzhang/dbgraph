import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://zaenzhang.github.io',
  base: '/dbgraph',
  integrations: [
    starlight({
      title: 'DbGraph',
      description: 'Local-first database context for AI coding agents.',
      customCss: ['./src/styles/custom.css'],
      locales: {
        root: {
          label: 'English',
          lang: 'en',
        },
        zh: {
          label: '简体中文',
          lang: 'zh-CN',
        },
      },
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/zaenzhang/dbgraph',
        },
      ],
      editLink: {
        baseUrl: 'https://github.com/zaenzhang/dbgraph/edit/master/site/src/content/docs/',
      },
      sidebar: [
        {
          label: 'Start',
          translations: { zh: '开始' },
          items: [
            { slug: 'index', label: 'Overview', translations: { zh: '概览' } },
            { slug: 'quickstart', label: 'Quickstart', translations: { zh: '快速开始' } },
            { slug: 'usage', label: 'Usage Guide', translations: { zh: '使用指南' } },
          ],
        },
        {
          label: 'Reference',
          translations: { zh: '参考' },
          items: [
            { slug: 'configuration', label: 'Configuration', translations: { zh: '配置' } },
            { slug: 'providers', label: 'Provider Capabilities', translations: { zh: '数据库支持' } },
            { slug: 'security', label: 'Security', translations: { zh: '安全' } },
            { slug: 'release-matrix', label: 'Release Matrix', translations: { zh: '发布矩阵' } },
          ],
        },
        {
          label: 'Proof',
          translations: { zh: '价值验证' },
          items: [
            { slug: 'agent-benchmark', label: 'Agent Benchmark', translations: { zh: 'Agent Benchmark' } },
          ],
        },
      ],
    }),
  ],
});
