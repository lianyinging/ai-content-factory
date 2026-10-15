import { invoke } from '@tauri-apps/api/core'

export type ProductItem = {
  id: string
  sku: string
  name: string
  spec: string
  description: string
  category: string
  unit: string
  image: string
  imageAbsPath: string
  source: 'seed' | 'user' | string
}

export async function productList() {
  return invoke<ProductItem[]>('product_list')
}

export async function productUpsert(body: {
  sku?: string
  name: string
  spec?: string
  description?: string
  category?: string
  unit?: string
  imageBase64?: string
  imagePath?: string
}) {
  return invoke<ProductItem>('product_upsert', { body })
}

export async function productDelete(sku: string) {
  return invoke<{ ok: boolean; sku: string }>('product_delete', { sku })
}
