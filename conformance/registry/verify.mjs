import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, isAbsolute } from 'node:path'

const [binary, mode] = process.argv.slice(2)
assert.ok(isAbsolute(binary)); assert.ok(['core', 'durable'].includes(mode))
function hash(parts) {
  const h = createHash('sha256')
  for (const part of parts) {
    const bytes = Buffer.from(part); const length = Buffer.alloc(8)
    length.writeBigUInt64BE(BigInt(bytes.length)); h.update(length); h.update(bytes)
  }
  return h.digest('hex')
}
const zero = Buffer.alloc(8); const one = Buffer.alloc(8); one.writeBigUInt64BE(1n)
const content = hash(['content/1', 'canonical payload'])
const prepared = hash(['prepared/1', 'standalone', 'first', zero, '', content])
const commit = hash(['commit/1', prepared, one])
const expected = `${prepared}:${commit}:1:second payload\n`
function run(args) {
  const result = spawnSync(binary, args, { encoding: 'utf8', timeout: 15000, maxBuffer: 65536 })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.stderr, '')
  assert.equal(result.stdout, expected)
  return result.stdout
}
run(['memory'])
if (mode === 'durable') {
  const directory = mkdtempSync(join(tmpdir(), 'zixcel-commit-process-'))
  try {
    const path = join(directory, 'commit.redb')
    assert.equal(run(['create', path]), run(['read', path]))
    assert.equal(run(['read', path]), expected)
    const absent = join(directory, 'missing.redb')
    const fail = spawnSync(binary, ['read', absent], { encoding: 'utf8', timeout: 15000, maxBuffer: 65536 })
    assert.notEqual(fail.status, 0); assert.equal(existsSync(absent), false)
  } finally { rmSync(directory, { recursive: true }) }
}
process.stdout.write(JSON.stringify({ status: 'PASS', mode, expected, independentDigest: true, separateProcesses: mode === 'durable' ? 5 : 1 }) + '\n')
