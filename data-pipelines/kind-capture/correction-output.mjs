import {
  chmodSync,
  closeSync,
  constants as fsConstants,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
  rmSync,
  rmdirSync,
} from 'node:fs';
import { join } from 'node:path';

const EXPECTED_OUTPUT_FILES = ['viewer.html', 'capture.json'];

export function reserveCorrectionOutputDirectory(output) {
  const namedOutputPath = join(output.anchoredParentPath, output.outputName);

  // mkdir is atomic and fails with EEXIST. Unlike rename(2), it never replaces
  // an empty directory created by another process between inspection and use.
  mkdirSync(namedOutputPath, { mode: 0o700 });

  let outputDirectoryFd;
  try {
    const namedStat = lstatSync(namedOutputPath);
    if (!namedStat.isDirectory() || namedStat.isSymbolicLink()) {
      throw new Error('reserved output is not a real directory');
    }
    outputDirectoryFd = openSync(
      namedOutputPath,
      fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW,
    );
    const openedStat = fstatSync(outputDirectoryFd);
    if (
      !openedStat.isDirectory() ||
      openedStat.dev !== namedStat.dev ||
      openedStat.ino !== namedStat.ino
    ) {
      throw new Error('reserved output changed while it was opened');
    }
    chmodSync(`/proc/self/fd/${outputDirectoryFd}`, 0o700);
    return {
      ...output,
      namedOutputPath,
      outputDirectoryFd,
      anchoredOutputPath: `/proc/self/fd/${outputDirectoryFd}`,
      outputDevice: openedStat.dev,
      outputInode: openedStat.ino,
    };
  } catch (error) {
    if (outputDirectoryFd !== undefined) closeSync(outputDirectoryFd);
    // The name may have been exchanged after mkdir. Without a successfully
    // pinned descriptor, removing it could delete another process's path.
    throw error;
  }
}

export function assertCurrentCorrectionOutputDirectory(output) {
  const currentParent = lstatSync(output.parentPath);
  const namedStat = lstatSync(output.namedOutputPath);
  const openedStat = fstatSync(output.outputDirectoryFd);
  if (
    !currentParent.isDirectory() ||
    currentParent.isSymbolicLink() ||
    currentParent.dev !== output.parentDevice ||
    currentParent.ino !== output.parentInode ||
    !namedStat.isDirectory() ||
    namedStat.isSymbolicLink() ||
    namedStat.dev !== output.outputDevice ||
    namedStat.ino !== output.outputInode ||
    !openedStat.isDirectory() ||
    openedStat.dev !== output.outputDevice ||
    openedStat.ino !== output.outputInode
  ) {
    throw new Error('reserved output directory changed');
  }
}

export function cleanupCorrectionOutputDirectory(output) {
  let cleaned = true;
  for (const name of EXPECTED_OUTPUT_FILES) {
    try {
      rmSync(join(output.anchoredOutputPath, name), { force: true });
    } catch {
      cleaned = false;
    }
  }
  try {
    const current = lstatSync(output.namedOutputPath);
    if (
      !current.isDirectory() ||
      current.isSymbolicLink() ||
      current.dev !== output.outputDevice ||
      current.ino !== output.outputInode
    ) {
      cleaned = false;
    } else {
      // Never recurse here. Unexpected entries make cleanup fail closed rather
      // than allowing this process to delete another actor's files.
      rmdirSync(output.namedOutputPath);
    }
  } catch {
    cleaned = false;
  }
  return cleaned;
}
