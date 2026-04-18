/**
 * @author: oldj
 * @homepage: https://oldj.net
 */

import { http_api_port } from '@common/constants'
import { serve } from '@hono/node-server'
import type { Context, Next } from 'hono'
import { Hono } from 'hono'
import * as net from 'net'
import api_router from './api/index'

export const app = new Hono()

export const requestLogger = async (c: Context, next: Next) => {
  const url = new URL(c.req.url)

  console.log(
    `> "${new Date().toString()}"`,
    c.req.method,
    `${url.pathname}${url.search}`,
    `"${c.req.header('user-agent')}"`,
  )
  await next()
}

export const homeHandler = (c: Context) => c.text('Hello SwitchHosts!')

export const remoteTestHandler = (c: Context) => c.text(`# remote-test\n# ${new Date().toString()}`)

const checkPortAvailable = (port: number, host: string): Promise<boolean> => {
  return new Promise((resolve) => {
    const server = net.createServer()
    server.once('error', () => resolve(false))
    server.once('listening', () => {
      server.close()
      resolve(true)
    })
    server.listen(port, host)
  })
}

const findAvailablePort = async (start_port: number, host: string): Promise<number> => {
  for (let port = start_port; port < start_port + 100; port++) {
    if (await checkPortAvailable(port, host)) {
      return port
    }
  }
  return 0
}

app.use('*', requestLogger)

app.get('/', homeHandler)

app.get('/remote-test', remoteTestHandler)

app.route('/api', api_router)

let server: ReturnType<typeof serve> | undefined
let current_port: number = http_api_port

export const getCurrentPort = (): number => current_port

export const start = async (http_api_only_local: boolean): Promise<boolean> => {
  let listenIp = http_api_only_local ? '127.0.0.1' : '0.0.0.0'

  const is_available = await checkPortAvailable(http_api_port, listenIp)

  if (!is_available) {
    console.log(`Port ${http_api_port} is already in use, trying to find an available port...`)
    const available_port = await findAvailablePort(http_api_port + 1, listenIp)

    if (available_port === 0) {
      console.error('No available port found!')
      return false
    }

    current_port = available_port
    console.log(`Using port ${available_port} instead of ${http_api_port}`)
  } else {
    current_port = http_api_port
  }

  try {
    server = serve(
      {
        fetch: app.fetch,
        port: current_port,
        hostname: listenIp,
      },
      () => {
        console.log(`SwitchHosts HTTP server is listening on port ${current_port}!`)
        console.log(`-> http://${listenIp}:${current_port}`)
      },
    )
  } catch (e) {
    console.error(e)
    return false
  }

  return true
}

export const stop = () => {
  if (!server) return

  try {
    server.close()
    server = undefined
  } catch (e) {
    console.error(e)
  }
}
