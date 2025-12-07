import Fastify from 'fastify'

const server = Fastify({ logger: true })

server.get('/health', async () => ({ ok: true }))

server.get('/api/hello', async (request, reply) => {
  return { message: 'Hello from Procurator API' }
})

const start = async () => {
  try {
    await server.listen({ port: 3000, host: '0.0.0.0' })
    server.log.info('Server listening')
  } catch (err) {
    server.log.error(err)
    process.exit(1)
  }
}

start()
