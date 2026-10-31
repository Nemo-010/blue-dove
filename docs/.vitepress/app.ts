/** The app's identity, shared by the site config and the theme. */
export const app = {
  name: 'Blue Dove',
  slug: 'blue-dove',
  description: 'A fast music player with an album browser and waveform seekbar',
  accent: '#f2a33c',
  /** Whether releases carry an AppImage: `appimage` in Cargo.toml's [package.metadata.pkgforge]. */
  appimage: true,
  get repo() {
    return `https://github.com/pkgforge-apps/${this.slug}`
  },
}
