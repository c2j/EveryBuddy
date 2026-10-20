import { createServer } from 'node:http'

export default {
  inject: ['fs'],
  apply(ctx) {
    const fs = ctx.fs
    const server = createServer(async (req, res) => {
      try {
        const url = new URL(req.url, 'http://127.0.0.1')
        if (url.pathname === '/health') {
          res.end('ok')
          return
        }
        if (url.pathname === '/list') {
          const root = await fs.resolve(url.searchParams.get('path') || '.', undefined)
          const entries = await fs.listDir(root)
          res.setHeader('content-type', 'application/json')
          res.end(JSON.stringify(entries.map((e) => ({ name: e.name, type: e.type }))))
          return
        }
        if (url.pathname === '/read') {
          const target = await fs.resolve(url.searchParams.get('path') || '.', undefined)
          res.setHeader('content-type', 'text/plain; charset=utf-8')
          res.end(await fs.readText(target))
          return
        }
        res.statusCode = 404
        res.end('not found')
      } catch (error) {
        res.statusCode = 500
        res.end(String(error?.message || error))
      }
    })
    server.listen(39471, '127.0.0.1')
    return () => server.close()
  },
}
