import type { ArchiveEntrySummary } from './archive-protocol'

export type ArchiveTreeNode = ArchiveEntrySummary & {
  name: string
  children: ArchiveTreeNode[]
}

type MutableArchiveTreeNode = ArchiveTreeNode & {
  childMap: Map<string, MutableArchiveTreeNode>
}

export function buildArchiveTree(entries: ArchiveEntrySummary[]): ArchiveTreeNode[] {
  const root = new Map<string, MutableArchiveTreeNode>()

  for (const entry of entries) {
    const segments = entry.path.split('/').filter(Boolean)
    if (segments.length === 0) continue
    let siblings = root
    let parentPath = ''

    for (let index = 0; index < segments.length; index += 1) {
      const name = segments[index]
      const path = parentPath ? `${parentPath}/${name}` : name
      const isLeaf = index === segments.length - 1
      const directory = !isLeaf || entry.directory
      let node = siblings.get(name)
      if (!node) {
        node = { path, name, directory, size: directory ? 0 : entry.size, children: [], childMap: new Map() }
        siblings.set(name, node)
      } else if (isLeaf && !entry.directory) {
        node.directory = false
        node.size = entry.size
      }
      parentPath = path
      siblings = node.childMap
    }
  }

  return finalizeNodes(root)
}

export function archiveFolderPaths(nodes: ArchiveTreeNode[]): string[] {
  const paths: string[] = []
  const visit = (items: ArchiveTreeNode[]) => {
    for (const item of items) {
      if (!item.directory) continue
      paths.push(item.path)
      visit(item.children)
    }
  }
  visit(nodes)
  return paths
}

function finalizeNodes(nodes: Map<string, MutableArchiveTreeNode>): ArchiveTreeNode[] {
  return [...nodes.values()]
    .sort((left, right) => Number(right.directory) - Number(left.directory) || left.name.localeCompare(right.name))
    .map(({ childMap, ...node }) => ({ ...node, children: finalizeNodes(childMap) }))
}
