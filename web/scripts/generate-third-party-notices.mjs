import { createHash } from 'node:crypto'
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises'
import { dirname, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const lockPath = resolve(webRoot, 'package-lock.json')
const nodeModulesRoot = resolve(webRoot, 'node_modules')
const noticePath = resolve(webRoot, 'public', 'THIRD_PARTY_NOTICES.txt')
const maxLicenseBytes = 1024 * 1024
const licenseName = /^licen[cs]e(?:[._-].+)?$/iu
const noticeName = /^notice(?:[._-].+)?$/iu
const readmeLicenseExceptions = new Map([
  [
    'node_modules/lru_map|lru_map@0.4.1',
    {
      filename: 'README.md',
      heading: '# MIT license\n',
      sha256: 'd444ac3606db5ac9790b4aa809172b9f7d88fe7a0093a1b01fd0abefb277dc05',
    },
  ],
])

function compareText(left, right) {
  return left < right ? -1 : left > right ? 1 : 0
}

function packageNameFromPath(packagePath) {
  const parts = packagePath.split('/')
  const nodeModulesIndex = parts.lastIndexOf('node_modules')
  if (nodeModulesIndex < 0 || nodeModulesIndex === parts.length - 1) {
    throw new Error(`invalid package-lock package path: ${packagePath}`)
  }

  const first = parts[nodeModulesIndex + 1]
  if (first.startsWith('@')) {
    if (nodeModulesIndex + 2 !== parts.length - 1) {
      throw new Error(`invalid scoped package-lock package path: ${packagePath}`)
    }
    return `${first}/${parts[nodeModulesIndex + 2]}`
  }
  if (nodeModulesIndex + 1 !== parts.length - 1) {
    throw new Error(`invalid package-lock package path: ${packagePath}`)
  }
  return first
}

function normalizeText(text) {
  const normalized = text.replace(/\r\n?/gu, '\n').replace(/\n+$/u, '')
  return `${normalized}\n`
}

async function readNoticeFile(packageName, packageDirectory, filename) {
  const bytes = await readFile(resolve(packageDirectory, filename))
  if (bytes.length > maxLicenseBytes) {
    throw new Error(
      `${packageName} ${filename} exceeds the ${maxLicenseBytes}-byte notice-file limit`,
    )
  }
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  if (text.trim().length === 0) {
    throw new Error(`${packageName} ${filename} is empty`)
  }
  return normalizeText(text)
}

async function readLicenseException(packageName, packageDirectory, exception) {
  const bytes = await readFile(resolve(packageDirectory, exception.filename))
  const sha256 = createHash('sha256').update(bytes).digest('hex')
  if (sha256 !== exception.sha256) {
    throw new Error(
      `${packageName} ${exception.filename} digest mismatch: expected ${exception.sha256}, found ${sha256}`,
    )
  }
  const text = normalizeText(new TextDecoder('utf-8', { fatal: true }).decode(bytes))
  const headingOffset = text.indexOf(exception.heading)
  if (headingOffset < 0 || text.indexOf(exception.heading, headingOffset + 1) >= 0) {
    throw new Error(
      `${packageName} ${exception.filename} must contain exactly one ${JSON.stringify(exception.heading.trim())} heading`,
    )
  }
  return {
    filename: `${exception.filename} (license section; explicit package exception)`,
    text: text.slice(headingOffset),
  }
}

async function productionPackages(lock) {
  if (lock['lockfileVersion'] !== 3) {
    throw new Error(`expected package-lock version 3, found ${lock['lockfileVersion']}`)
  }
  const packages = lock['packages']
  if (typeof packages !== 'object' || packages === null || Array.isArray(packages)) {
    throw new Error('package-lock packages must be an object')
  }
  if (typeof packages[''] !== 'object' || packages[''] === null) {
    throw new Error('package-lock is missing the root package entry')
  }

  const records = []
  const consumedExceptions = new Set()
  for (const [packagePath, lockedPackage] of Object.entries(packages)) {
    if (packagePath === '' || lockedPackage['dev'] === true) {
      continue
    }
    if (lockedPackage['link'] === true) {
      throw new Error(`linked production dependency is unsupported: ${packagePath}`)
    }
    if (!packagePath.startsWith('node_modules/')) {
      throw new Error(`production dependency is outside node_modules: ${packagePath}`)
    }

    const packageName = packageNameFromPath(packagePath)
    const packageDirectory = resolve(webRoot, packagePath)
    if (
      packageDirectory !== nodeModulesRoot &&
      !packageDirectory.startsWith(`${nodeModulesRoot}${sep}`)
    ) {
      throw new Error(`production dependency escapes node_modules: ${packagePath}`)
    }

    const manifest = JSON.parse(
      await readFile(resolve(packageDirectory, 'package.json'), 'utf8'),
    )
    if (manifest['name'] !== packageName) {
      throw new Error(
        `${packagePath} package name mismatch: lock path names ${packageName}, installed manifest names ${manifest['name']}`,
      )
    }
    if (manifest['version'] !== lockedPackage['version']) {
      throw new Error(
        `${packageName} version mismatch: lock has ${lockedPackage['version']}, installed manifest has ${manifest['version']}`,
      )
    }
    if (
      typeof lockedPackage['license'] !== 'string' ||
      lockedPackage['license'].length === 0
    ) {
      throw new Error(`${packageName}@${manifest['version']} has no license in package-lock.json`)
    }
    if (manifest['license'] !== lockedPackage['license']) {
      throw new Error(
        `${packageName}@${manifest['version']} license mismatch: lock has ${lockedPackage['license']}, installed manifest has ${manifest['license']}`,
      )
    }

    const entries = await readdir(packageDirectory, { withFileTypes: true })
    const licenseFiles = entries
      .filter((entry) => entry.isFile() && licenseName.test(entry.name))
      .map((entry) => entry.name)
      .sort(compareText)
    const noticeFiles = entries
      .filter((entry) => entry.isFile() && noticeName.test(entry.name))
      .map((entry) => entry.name)
      .sort(compareText)
    const files = []
    if (licenseFiles.length === 0) {
      const exceptionKey = `${packagePath}|${packageName}@${manifest['version']}`
      const exception = readmeLicenseExceptions.get(exceptionKey)
      if (exception === undefined) {
        throw new Error(`${packageName}@${manifest['version']} has no LICENSE file`)
      }
      files.push(await readLicenseException(packageName, packageDirectory, exception))
      consumedExceptions.add(exceptionKey)
    }
    for (const filename of [...licenseFiles, ...noticeFiles]) {
      files.push({
        filename,
        text: await readNoticeFile(packageName, packageDirectory, filename),
      })
    }
    records.push({
      license: lockedPackage['license'],
      name: packageName,
      packagePath,
      version: manifest['version'],
      files,
    })
  }

  for (const exceptionKey of readmeLicenseExceptions.keys()) {
    if (!consumedExceptions.has(exceptionKey)) {
      throw new Error(`unused README license exception: ${exceptionKey}`)
    }
  }

  records.sort((left, right) =>
    compareText(left.name, right.name) ||
    compareText(left.version, right.version) ||
    compareText(left.packagePath, right.packagePath),
  )
  return records
}

function renderNotices(records, lockSha256) {
  const output = [
    'StrataDiff Evidence Workbench Third-Party Notices',
    '',
    'This file is generated from web/package-lock.json and installed package license files.',
    'Do not edit it by hand.',
    '',
    `package-lock.json SHA-256: ${lockSha256}`,
    `Production package entries: ${records.length}`,
    '',
  ]

  for (const record of records) {
    output.push('='.repeat(80))
    output.push(`${record.name}@${record.version}`)
    output.push(`Installed path: ${record.packagePath}`)
    output.push(`Declared license: ${record.license}`)
    output.push('')
    for (const file of record.files) {
      output.push(`--- ${file.filename} ---`)
      output.push(file.text.replace(/\n$/u, ''))
      output.push('')
    }
  }

  return `${output.join('\n').replace(/\n+$/u, '')}\n`
}

async function main() {
  const arguments_ = process.argv.slice(2)
  if (
    arguments_.length !== 1 ||
    (arguments_[0] !== '--write' && arguments_[0] !== '--check')
  ) {
    throw new Error('usage: node scripts/generate-third-party-notices.mjs --write|--check')
  }

  const lockBytes = await readFile(lockPath)
  const lock = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(lockBytes))
  const records = await productionPackages(lock)
  const lockSha256 = createHash('sha256').update(lockBytes).digest('hex')
  const generated = renderNotices(records, lockSha256)

  if (arguments_[0] === '--write') {
    await mkdir(dirname(noticePath), { recursive: true })
    await writeFile(noticePath, generated, 'utf8')
    console.log(`wrote ${relative(webRoot, noticePath)} with ${records.length} package entries`)
    return
  }

  const committed = await readFile(noticePath, 'utf8')
  if (committed !== generated) {
    throw new Error(
      'third-party notices do not match package-lock.json and node_modules; run npm run notices:generate',
    )
  }
  console.log(`verified ${relative(webRoot, noticePath)} with ${records.length} package entries`)
}

await main()
