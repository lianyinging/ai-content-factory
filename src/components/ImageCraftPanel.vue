<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { aiOptimizePrompt } from '@/api/ai'
import { mediaDisplaySrc } from '@/api/assets'
import { imageJobStart } from '@/api/imageJobs'
import { useImageJobsStore } from '@/state/imageJobs'

const props = withDefaults(
  defineProps<{
    draftId?: string
    folderId?: string
    platform?: string
    /** 外部指定底图（素材中心拖入） */
    externalBaseUrl?: string
    externalBaseName?: string
    /** 拖拽悬停高亮 */
    baseDropActive?: boolean
  }>(),
  {
    draftId: '',
    folderId: '',
    platform: '',
    externalBaseUrl: '',
    externalBaseName: '',
    baseDropActive: false,
  },
)

const emit = defineEmits<{ started: [] }>()

const IMAGE_RATIOS = [
  { label: '1:1', size: '1024x1024' },
  { label: '3:4', size: '1024x1536' },
  { label: '16:9', size: '1536x1024' },
] as const

const IMAGE_QUALITIES = [
  { value: 'low', label: '标准' },
  { value: 'medium', label: '高清' },
  { value: 'high', label: '超清' },
] as const

const JEWELRY_WEAR_PRESETS = [
  {
    label: '手模特写',
    prompt:
      '将画面中的首饰改为一枚戴在优雅女性手指上的展示图，手部特写，手指纤细白皙，自然手势，柔和棚拍光，浅景深，保留首饰原有材质、颜色与细节，无文字水印，小红书种草风',
  },
  {
    label: '耳部佩戴',
    prompt:
      '将画面中的耳饰改为佩戴在女性侧脸耳部的实拍效果，露出耳廓与少量发丝，轻妆，柔和自然光，背景虚化，保留耳饰原设计与光泽，高清细节，无文字',
  },
  {
    label: '颈间佩戴',
    prompt:
      '将画面中的项链改为佩戴在女性锁骨颈间的特写，肌肤细腻，锁骨线条自然，轻柔侧光，高级感，保留项链造型与质感，浅景深，无文字水印',
  },
  {
    label: '手腕佩戴',
    prompt:
      '将画面中的手链/手镯改为戴在女性纤细手腕上的特写，手势自然放松，肤色干净，柔光棚拍，保留原有材质与细节，背景简洁，无文字',
  },
  {
    label: '对戒/叠戴',
    prompt:
      '将画面中的戒指改为女性双手交叠佩戴的特写，可轻微叠戴感，指尖干净，温柔自然光，保留戒托与宝石原貌，小红书产品种草风，无文字',
  },
  {
    label: '试戴自拍感',
    prompt:
      '将画面中的首饰改为近距离手机自拍视角的试戴效果，真人感但不露完整五官，聚焦佩戴部位，生活感光线，保留首饰真实细节，适合小红书笔记配图，无文字水印',
  },
] as const

const craftMode = ref<'generate' | 'edit'>('generate')
const prompt = ref('')
const imageSize = ref('1024x1024')
const imageQuality = ref('medium')
const baseImageUrl = ref('')
const baseImageName = ref('')
const baseFileInput = ref<HTMLInputElement | null>(null)
const submitting = ref(false)
const optimizing = ref(false)

const store = useImageJobsStore()
const isEdit = computed(() => craftMode.value === 'edit')
const busy = computed(() => submitting.value || optimizing.value)

watch(
  () => [props.externalBaseUrl, props.externalBaseName] as const,
  ([url, name]) => {
    if (url) {
      baseImageUrl.value = url
      baseImageName.value = name || '底图'
      craftMode.value = 'edit'
    }
  },
  { immediate: true },
)

function clearBase() {
  baseImageUrl.value = ''
  baseImageName.value = ''
  if (baseFileInput.value) baseFileInput.value.value = ''
}

function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result || ''))
    reader.onerror = () => reject(new Error('读取底图失败'))
    reader.readAsDataURL(file)
  })
}

async function onBaseFileChange(ev: Event) {
  const input = ev.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  if (!file.type.startsWith('image/')) {
    ElMessage.warning('请选择图片文件')
    return
  }
  try {
    baseImageUrl.value = await readFileAsDataUrl(file)
    baseImageName.value = file.name || '本地底图'
    craftMode.value = 'edit'
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function toAiSource(src: string) {
  const s = String(src || '').trim()
  if (!s) return s
  if (s.startsWith('data:') || s.startsWith('file:') || s.startsWith('/')) return s
  if (/^[A-Za-z]:[\\/]/.test(s)) return s
  try {
    const u = new URL(s)
    if (u.hostname === 'asset.localhost' || u.protocol === 'asset:') {
      return decodeURIComponent(u.pathname || '')
    }
  } catch {
    /* keep */
  }
  return s
}

async function onAiOptimize() {
  const draft = prompt.value.trim()
  if (!draft) {
    ElMessage.warning(isEdit.value ? '请先填写编辑指令草稿' : '请先填写提示词草稿')
    return
  }
  if (isEdit.value && !baseImageUrl.value) {
    ElMessage.warning('图生图优化需要底图，请先选择或上传')
    return
  }
  if (busy.value) return
  optimizing.value = true
  try {
    const kind = isEdit.value ? 'image-edit' : 'image-generate'
    const res = await aiOptimizePrompt({
      kind,
      prompt: draft,
      styleHint: isEdit.value ? 'image edit instruction' : '配图提示词',
      imageUrl: isEdit.value ? toAiSource(baseImageUrl.value) : undefined,
    })
    prompt.value = res.prompt
    if (isEdit.value && res.usedVision) {
      ElMessage.success('已结合底图理解并优化')
    } else if (isEdit.value) {
      ElMessage.warning('看图优化未成功，已用文本模型润色指令')
    } else {
      ElMessage.success('已应用 AI 优化结果')
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : 'AI 优化失败')
  } finally {
    optimizing.value = false
  }
}

async function onSubmit() {
  const p = prompt.value.trim()
  if (!p) {
    ElMessage.warning(isEdit.value ? '请填写编辑指令' : '请填写配图提示词')
    return
  }
  if (isEdit.value && !baseImageUrl.value) {
    ElMessage.warning('请先选择或上传底图')
    return
  }
  if (busy.value) return
  submitting.value = true
  try {
    const job = await imageJobStart({
      kind: isEdit.value ? 'edit' : 'generate',
      prompt: p,
      size: imageSize.value,
      quality: imageQuality.value,
      baseImagePath: isEdit.value ? toAiSource(baseImageUrl.value) : undefined,
      draftId: props.draftId || undefined,
      folderId: props.folderId || undefined,
      platform: props.platform || undefined,
    })
    store.upsertLocal(job)
    ElMessage.success('已提交生成任务，可继续浏览其它页面')
    emit('started')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="craft">
    <div class="craft__seg">
      <button
        type="button"
        class="craft__seg-btn"
        :class="{ 'is-on': craftMode === 'generate' }"
        @click="craftMode = 'generate'"
      >
        文生图
      </button>
      <button
        type="button"
        class="craft__seg-btn"
        :class="{ 'is-on': craftMode === 'edit' }"
        @click="craftMode = 'edit'"
      >
        图生图
      </button>
    </div>

    <div
      v-show="isEdit || baseDropActive"
      class="craft__base"
      :class="{ 'is-drop': baseDropActive }"
      data-craft-base-drop="1"
    >
      <div class="craft__base-head">
        <label>底图</label>
        <div class="craft__base-actions">
          <button type="button" class="craft__link" @click="baseFileInput?.click()">上传</button>
          <button v-if="baseImageUrl" type="button" class="craft__link" @click="clearBase">清除</button>
        </div>
      </div>
      <input
        ref="baseFileInput"
        type="file"
        accept="image/*"
        class="craft__file"
        @change="onBaseFileChange"
      />
      <div v-if="baseImageUrl" class="craft__base-card">
        <img :src="mediaDisplaySrc(baseImageUrl)" alt="" referrerpolicy="no-referrer" />
        <div>
          <strong>{{ baseImageName || '底图' }}</strong>
          <em>可从右侧拖入图片替换</em>
        </div>
      </div>
      <p v-else class="craft__hint">上传底图，或从右侧拖入图片到此处</p>
    </div>

    <div class="craft__field">
      <div class="craft__field-head">
        <label>{{ isEdit ? '编辑指令' : '正向提示词' }}</label>
        <button
          type="button"
          class="craft__ai-opt"
          :disabled="busy || (isEdit && !baseImageUrl)"
          @click="onAiOptimize"
        >
          {{ optimizing ? '优化中…' : '✨ AI 优化' }}
        </button>
      </div>
      <el-input
        v-model="prompt"
        type="textarea"
        :rows="4"
        :disabled="busy"
        :placeholder="isEdit ? '描述要如何修改这张图，例如：手模佩戴特写…' : '描述想要的配图风格与主体'"
      />
      <div v-if="isEdit" class="craft__chips craft__wear-presets">
        <button
          v-for="p in JEWELRY_WEAR_PRESETS"
          :key="p.label"
          type="button"
          class="craft__chip"
          :class="{ 'is-on': prompt === p.prompt }"
          :disabled="busy"
          :title="p.prompt"
          @click="prompt = p.prompt"
        >
          {{ p.label }}
        </button>
      </div>
    </div>

    <div class="craft__field">
      <label>比例 / 尺寸</label>
      <div class="craft__chips">
        <button
          v-for="r in IMAGE_RATIOS"
          :key="r.size"
          type="button"
          class="craft__chip"
          :class="{ 'is-on': imageSize === r.size }"
          @click="imageSize = r.size"
        >
          {{ r.label }}
        </button>
      </div>
    </div>

    <div class="craft__field">
      <label>质量</label>
      <div class="craft__chips">
        <button
          v-for="q in IMAGE_QUALITIES"
          :key="q.value"
          type="button"
          class="craft__chip"
          :class="{ 'is-on': imageQuality === q.value }"
          @click="imageQuality = q.value"
        >
          {{ q.label }}
        </button>
      </div>
    </div>

    <el-button
      type="primary"
      style="width: 100%"
      :loading="submitting"
      :disabled="busy || (isEdit && !baseImageUrl)"
      @click="onSubmit"
    >
      生成配图
    </el-button>
    <p class="craft__note">提交后在后台执行，可离开本页；完成后会在素材中心与草稿生成结果中出现。</p>
  </div>
</template>

<style scoped lang="scss">
.craft {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.craft__seg {
  display: flex;
  gap: 6px;
  padding: 4px;
  border-radius: 10px;
  background: var(--color-fill-2, #f2f4f7);
}
.craft__seg-btn {
  flex: 1;
  height: 30px;
  border: none;
  border-radius: 8px;
  background: transparent;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  color: var(--color-text-2, #4e5969);
  &.is-on {
    background: #fff;
    color: var(--el-color-primary);
    box-shadow: 0 1px 2px rgba(15, 23, 42, 0.08);
  }
}
.craft__base {
  padding: 10px;
  border: 1px dashed var(--color-border-2, #e8eaee);
  border-radius: 10px;
  background: var(--color-fill-1, #f7f8fa);
  transition:
    border-color 0.12s ease,
    background 0.12s ease;

  &.is-drop {
    border-color: var(--el-color-primary);
    background: color-mix(in srgb, var(--el-color-primary) 10%, #fff);
  }
}
.craft__base-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
  label {
    font-size: 12px;
    font-weight: 600;
  }
}
.craft__base-actions {
  display: flex;
  gap: 10px;
}
.craft__link {
  border: none;
  background: none;
  color: var(--el-color-primary);
  font-size: 12px;
  cursor: pointer;
  padding: 0;
}
.craft__file {
  display: none;
}
.craft__base-card {
  display: flex;
  gap: 10px;
  align-items: center;
  img {
    width: 56px;
    height: 74px;
    object-fit: cover;
    border-radius: 6px;
    border: 1px solid var(--color-border-2, #e8eaee);
  }
  strong {
    display: block;
    font-size: 13px;
  }
  em {
    display: block;
    margin-top: 2px;
    font-size: 11px;
    font-style: normal;
    color: var(--color-text-3, #8a919f);
  }
}
.craft__hint,
.craft__note {
  margin: 0;
  font-size: 12px;
  line-height: 1.45;
  color: var(--color-text-3, #8a919f);
}
.craft__field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  label {
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text-2, #4e5969);
  }
}
.craft__field-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.craft__ai-opt {
  border: none;
  background: none;
  padding: 0;
  font-size: 12px;
  font-weight: 600;
  color: var(--el-color-primary);
  cursor: pointer;
  &:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
}
.craft__chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.craft__wear-presets {
  margin-top: 8px;
}
.craft__chip {
  height: 28px;
  padding: 0 10px;
  border: 1px solid var(--color-border-2, #e8eaee);
  border-radius: 999px;
  background: #fff;
  font-size: 12px;
  cursor: pointer;
  &.is-on {
    border-color: var(--el-color-primary);
    color: var(--el-color-primary);
    background: color-mix(in srgb, var(--el-color-primary) 8%, #fff);
  }
}
</style>
