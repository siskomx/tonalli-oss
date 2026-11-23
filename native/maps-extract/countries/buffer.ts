// Adapted from Project NOMAD, Apache-2.0, c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Changes: exported pure buffering helper; standalone build-time generation only.
export function bufferGeometry(geometry: unknown, buffer: number): unknown {
  const geom = geometry as { type: string; coordinates: any }
  if (geom?.type === 'Polygon') {
    return { type: 'Polygon', coordinates: bufferPolygonRings(geom.coordinates, buffer) }
  }
  if (geom?.type === 'MultiPolygon') {
    return {
      type: 'MultiPolygon',
      coordinates: geom.coordinates.map((poly: number[][][]) =>
        bufferPolygonRings(poly, buffer)
      ),
    }
  }
  return geometry
}

function bufferPolygonRings(rings: number[][][], buffer: number): number[][][] {
  return rings.map((ring) => bufferRing(ring, buffer))
}

function bufferRing(ring: number[][], buffer: number): number[][] {
  if (ring.length < 4) return ring
  const sign = signedArea(ring) > 0 ? 1 : -1
  const n = ring.length - 1
  const out: number[][] = []
  for (let i = 0; i < n; i++) {
    const prev = ring[(i - 1 + n) % n]
    const curr = ring[i]
    const next = ring[(i + 1) % n]
    const e1x = curr[0] - prev[0]
    const e1y = curr[1] - prev[1]
    const e2x = next[0] - curr[0]
    const e2y = next[1] - curr[1]
    const l1 = Math.hypot(e1x, e1y) || 1
    const l2 = Math.hypot(e2x, e2y) || 1
    const n1x = (e1y / l1) * sign
    const n1y = (-e1x / l1) * sign
    const n2x = (e2y / l2) * sign
    const n2y = (-e2x / l2) * sign
    const sumX = n1x + n2x
    const sumY = n1y + n2y
    const sl = Math.hypot(sumX, sumY) || 1
    out.push([curr[0] + (sumX / sl) * buffer, curr[1] + (sumY / sl) * buffer])
  }
  out.push(out[0])
  return out
}

function signedArea(ring: number[][]): number {
  let a = 0
  for (let i = 0; i < ring.length - 1; i++) {
    a += ring[i][0] * ring[i + 1][1] - ring[i + 1][0] * ring[i][1]
  }
  return a / 2
}
