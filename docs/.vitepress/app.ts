/** The app's identity, shared by the site config and the theme. */
export const app = {
  name: 'Blue Dove',
  slug: 'blue-dove',
  description: 'A fast simple music player with no bloat, album browser and a waveform seekbar, preview mode and multiple theme options',
  accent: '#f2a33c',
  get repo() {
    return `https://github.com/pkgforge-apps/${this.slug}`
  },
}
