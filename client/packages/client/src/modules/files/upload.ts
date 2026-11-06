import { assertWritable } from '@penfii/api-client'
import type { FileObject } from '@penfii/contracts'

/** The server answered the upload with an error status. */
export class UploadHttpError extends Error {
  constructor(readonly status: number, message: string) {
    super(message)
    this.name = 'UploadHttpError'
  }
}

export function putFile(fileId: string, file: File, onProgress: (ratio: number) => void): {
  done: Promise<FileObject>
  abort: () => void
} {
  const xhr = new XMLHttpRequest()
  const done = new Promise<FileObject>((resolve, reject) => {
    try {
      assertWritable()
    } catch (cause) {
      reject(cause)
      return
    }
    xhr.open('PUT', `/v1/files/${fileId}/content`)
    xhr.withCredentials = true
    xhr.setRequestHeader('X-Penfii-Client', '1')
    xhr.setRequestHeader('Content-Type', 'application/octet-stream')
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable && event.total > 0) onProgress(event.loaded / event.total)
    }
    xhr.onload = () => {
      if (xhr.status < 200 || xhr.status >= 300) {
        reject(new UploadHttpError(xhr.status, xhr.responseText || `HTTP ${String(xhr.status)}`))
        return
      }
      resolve(JSON.parse(xhr.responseText) as FileObject)
    }
    xhr.onerror = () => reject(new Error('upload failed'))
    xhr.onabort = () => reject(new Error('aborted'))
    xhr.send(file)
  })
  return { done, abort: () => xhr.abort() }
}
