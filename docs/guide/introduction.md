# Introduction

Blue Dove is a music player for the directories your music sits in. Point it at
one or several of them, and it lists what is there, grouped into albums by the
tags the files already carry. There is no database to build, no files are copied
or moved, and nothing leaves the machine.

It replaces players such as Clementine, Strawberry and Elisa for
listening to a local library, without their size or their background
services. It does not stream, scrobble, sync devices or rip CDs.

## What it does

- Reads FLAC, MP3, M4A and ALAC, AAC, Ogg Vorbis, Opus, WAV, AIFF, CAF,
  Matroska audio, Monkey's Audio, WavPack, TTA, Musepack and Shorten, and
  the formats video game rips use: Sony VAG, CRI ADX, and Nintendo DSP,
  BRSTM, BCSTM and BFSTM. Every decoder is pure Rust, so nothing has to be
  installed to play them.
- Browses the music as a tree of directories and albums, which can be folded
  away, and queues an album or a whole directory into a playlist.
- Reads several directories as one library, and leaves a directory out of the
  scan without moving a file.
- Keeps several playlists open as tabs, each with its own name, and
  remembers them between runs. A playlist that holds more than one album
  is drawn as a list of albums, each with its own heading.
- Draws the waveform of the playing track and seeks anywhere in it, and
  keeps the waveform so a track played before draws at once.
- Selects several tracks at once, plays just those, or removes them from
  the playlist without touching the files.
- Shows the columns you want: number, title, file name, artist, album and
  length, each resizable and each able to be turned off.
- Shrinks into a compact popup of just the art, the controls and a thin
  waveform, and keeps playing when the window is closed if asked to.
- Answers MPRIS, so `playerctl` and the desktop's media keys drive it.

## What it does not do

- Editing tags, converting files or ripping CDs.
- Network access of any kind.

## Next

- [Installation](/guide/installation) has the files for each platform.
- [Using Blue Dove](/guide/using) walks through the window and the keyboard.
