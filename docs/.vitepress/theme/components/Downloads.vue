<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { app } from '../../app'

type Os = 'linux' | 'macos' | 'windows'

interface Asset {
  label: string
  file: string
  note?: string
}

interface Platform {
  os: Os
  label: string
  assets: Asset[]
}

const latest = `${app.repo}/releases/latest/download/${app.slug}`

const platforms: Platform[] = [
  {
    os: 'linux',
    label: 'Linux',
    assets: [
      { label: 'x86_64', file: `${latest}-x86_64-linux.onelf`, note: 'portable, one file' },
      { label: 'aarch64', file: `${latest}-aarch64-linux.onelf`, note: 'portable, one file' },
      ...(app.appimage
        ? [
            { label: 'x86_64', file: `${latest}-anylinux-x86_64.AppImage`, note: 'AppImage' },
            { label: 'aarch64', file: `${latest}-anylinux-aarch64.AppImage`, note: 'AppImage' },
          ]
        : []),
      { label: 'x86_64', file: `${latest}-x86_64-linux.tar.xz`, note: 'archive' },
      { label: 'aarch64', file: `${latest}-aarch64-linux.tar.xz`, note: 'archive' },
    ],
  },
  {
    os: 'macos',
    label: 'macOS',
    assets: [
      { label: 'Apple silicon', file: `${latest}-aarch64-macos.dmg` },
      { label: 'Intel', file: `${latest}-x86_64-macos.dmg` },
    ],
  },
  {
    os: 'windows',
    label: 'Windows',
    assets: [{ label: 'x86_64', file: `${latest}-x86_64-windows.exe` }],
  },
]

const detected = ref<Os | null>(null)

onMounted(() => {
  const ua = navigator.userAgent
  if (/Windows/.test(ua)) detected.value = 'windows'
  else if (/Macintosh|Mac OS X/.test(ua)) detected.value = 'macos'
  else if (/Linux|X11/.test(ua)) detected.value = 'linux'
})

const ordered = computed(() =>
  [...platforms].sort((a, b) => Number(b.os === detected.value) - Number(a.os === detected.value)),
)
</script>

<template>
  <div class="downloads">
    <section
      v-for="platform in ordered"
      :key="platform.os"
      class="platform"
      :class="{ current: platform.os === detected }"
    >
      <header>
        <h3>{{ platform.label }}</h3>
        <span v-if="platform.os === detected" class="badge">Your system</span>
      </header>
      <ul>
        <li v-for="asset in platform.assets" :key="asset.file">
          <a :href="asset.file">{{ asset.label }}</a>
          <span v-if="asset.note" class="note">{{ asset.note }}</span>
        </li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.downloads {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 16px;
  margin: 24px 0;
}

.platform {
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--app-radius);
  background: var(--vp-c-bg-soft);
  padding: 18px 20px;
  transition: border-color 0.2s, transform 0.2s;
}

.platform.current {
  border-color: var(--vp-c-brand-1);
  box-shadow: 0 0 0 3px var(--vp-c-brand-soft);
}

.platform:hover {
  transform: translateY(-2px);
}

header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}

h3 {
  margin: 0;
  font-size: 17px;
  font-weight: 700;
}

.badge {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--vp-c-brand-1);
  background: var(--vp-c-brand-soft);
  padding: 2px 8px;
  border-radius: 999px;
}

ul {
  list-style: none;
  margin: 0;
  padding: 0;
}

li {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  padding: 6px 0;
  border-top: 1px dashed var(--vp-c-divider);
}

li:first-child {
  border-top: none;
}

a {
  font-weight: 600;
  text-decoration: none;
}

.note {
  font-size: 13px;
  color: var(--vp-c-text-3);
}
</style>
