import { createRouter, createWebHistory } from 'vue-router'

export const routes = [
  {
    path: '/',
    name: 'overview',
    component: () => import('@/views/OverviewView.vue'),
  },
  {
    path: '/instances',
    name: 'instances',
    component: () => import('@/views/InstancesView.vue'),
  },
  {
    // The selected session belongs in the address: Refresh and Back have to come
    // back to the same transcript, and a handoff is worth being able to share.
    path: '/sessions/:sessionId?',
    name: 'sessions',
    component: () => import('@/views/SessionsView.vue'),
  },
  {
    path: '/failures',
    name: 'failures-list',
    component: () => import('@/views/FailuresView.vue'),
    // The drawer is a child of the list, so the URL stays the only way to open
    // it and the filtered rows stay mounted behind it (D5).
    children: [
      {
        path: ':traceId',
        name: 'failure-detail',
        component: () => import('@/views/FailureView.vue'),
      },
    ],
  },
]

export const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes,
})
