/**
 * ResourceList.java
 *
 * (as oppposed to ListResource)
 * 
 *  A list which will update trees automatically as items are added and removed.
 * 
 *  A ResourceFactory is used to turn objects into tree nodes
 *
 * @author Peter Halasz
 */

package net.pengo.restree;

import java.util.*;

import javax.swing.JTree;
import javax.swing.tree.DefaultMutableTreeNode;
import javax.swing.tree.MutableTreeNode;
import javax.swing.tree.TreePath;

import net.pengo.app.OpenFile;
import net.pengo.resource.Resource;
import net.pengo.resource.ResourceFactory;
import javax.swing.tree.DefaultTreeModel;

public class ResourceList implements List {
    String name;
    private List list;
    private ResourceFactory resFact;
    private Map parentTree = new HashMap(); // parent nodes for this resource list. MutableTreeNode->JTree. MutableTreeNode must not be modified by any other objects besides this.
    
    public ResourceList(List list) {
        this(list, (ResourceFactory)null, null);
    }
    
    public ResourceList(List list, ResourceFactory resFact, String name) {
        this.list = list;
        setResourceFactory(resFact);
        this.name = name;
    }
    
    public ResourceList(List list, OpenFile openFile, String name) {
        this.list = list;
        setResourceFactory(openFile);
        this.name = name;
    }
    
    // for your convinence
    public void setResourceFactory(OpenFile of) {
        if (of == null) {
            setResourceFactory((ResourceFactory)null);
        } else {
            setResourceFactory(of.getResourceFactory());
        }
    }
    
    public void setResourceFactory(ResourceFactory resFact) {
        //fixme: what about rebuilding the tree?
        if (resFact == null) {
            resFact = ResourceFactory.getDefault();
        }
        this.resFact = resFact;
    }
    
    public String toString() {
        if (name == null) {
            return super.toString();
        }
        
        return name;
    }
    
    public ListIterator listIterator() {
        return list.listIterator();
    }
    
    public Object get(int index) {
        return list.get(index);
    }
    
    public boolean isEmpty() {
        return list.isEmpty();
    }
    
    public boolean add(Object o) {
        boolean r = list.add(o);
        if (r) {
            addRes(list.size()-1, o);
        }
        
        return r;
    }
    
    public void add(int index, Object element) {
        list.add(index, element);
        addRes(index, element);
    }
    
    // does the work for the different add methods.
    private void addRes(int index, Object element) {
        for (Iterator i=parentTree.keySet().iterator(); i.hasNext(); ) {
            MutableTreeNode p = (MutableTreeNode)i.next();
            SimpleResTree jt = (SimpleResTree)parentTree.get(p);
            insertNode(p, element, jt, index);
        }
        //subTreeCheck(SimpleResTree parent, Object childObject, MutableTreeNode childNode) {
        //((ResourceList)childObject).addParent((SimpleResTree)parent.get(p), child); // i think
    }
    
    private void insertNode(MutableTreeNode parent, Object childObject, SimpleResTree jt, int index) {
        Resource resourceObject = resFact.wrap(childObject);
        DefaultTreeModel tm = (DefaultTreeModel)jt.getModel();
        DefaultMutableTreeNode child = new DefaultMutableTreeNode(resourceObject);
        
        //System.out.println("  adding resource: " + childObject + " to " + parent );
        
        //p.insert(child, index); // not much good.
        tm.insertNodeInto(child, parent, index);
        jt.makeVisible(new TreePath(((DefaultTreeModel)jt.getModel()).getPathToRoot(child))); //fixme messy? nah
        subTreeCheck(jt, childObject, child);
        //System.out.println("...added to " + parent);
        
    }
    
    private void subTreeCheckAll(Object childObject, MutableTreeNode childNode) {
        for (Iterator i=parentTree.values().iterator(); i.hasNext(); ) {
            SimpleResTree rt = (SimpleResTree)i.next();
            subTreeCheck(rt, childObject, childNode);
        }
    }
    
    private void subTreeCheck(SimpleResTree parent, Object childObject, MutableTreeNode childNode) {
        //fixme: what a mess!
        if (childObject instanceof ResourceList) {
            //System.out.println("...given new parent " + parent);
            ResourceList childResList = (ResourceList)childObject;
            childResList.addParent(parent, childNode);
        } else if (childObject instanceof ResourceSortedSet) {
            ResourceSortedSet childResList = (ResourceSortedSet)childObject;
            childResList.addParent(parent, childNode);
        } else if (childObject instanceof OpenFile) {
            ResourceList childResList = ((OpenFile)childObject).getResourceList(); //fixme: unused?
            childResList.addParent(parent, childNode);
        }
    }
    
    // used by SimpleResTree to attach ResourceLists to itself.
    // also used by other ResourceLists to form hierarchies
    public void addParent(SimpleResTree rt, MutableTreeNode p) {
        // add existing items to new parent node
        int i=0;
        for (Iterator it=list.iterator(); it.hasNext(); ) {
            Object obj = it.next();
            //DefaultTreeModel tm = (DefaultTreeModel)rt.getModel();
            //p.insert(child, i); // not much good.
            insertNode(p, obj, rt, i);
            i++;
        }
        
        // add new parent node to list of parent nodes
        parentTree.put(p, rt);
    }
    
    public List subList(int fromIndex, int toIndex) {
        return list.subList(fromIndex, toIndex);
    }
    
    public Object set(int index, Object element) {
        //fixme! cheap n dirty.
        Object o = remove(index);
        add(index, element);
        return o;
    }
    
    
    public Object[] toArray() {
        return list.toArray();
    }
    
    public int lastIndexOf(Object o) {
        return list.lastIndexOf(o);
    }
    
    public Object[] toArray(Object[] a) {
        return toArray(a);
    }
    
    public Iterator iterator() {
        return list.iterator();
    }
    
    public Object remove(int index) {
        removeRes(index);
        return list.remove(index);
    }
    
    public boolean remove(Object o) {
        int i = list.indexOf(o);
        if (i != -1) {
            removeRes(i);
        } else {
            System.out.println("could not remove: " + o);
        }
        return list.remove(o);
    }
    
    public void clear() {
        // remove all children from all parents.
        for (int i=list.size()-1; i>=0; i--) {
            remove(i);
        }
        
    }
    
    public boolean removeAll(Collection c) {
        //Fixme: nyi
        System.out.println("NYI: public boolean removeAll(Collection c)");
        return list.removeAll(c);
    }
    
    // does the work for the remove methods
    private void removeRes(int index) {
        //System.out.println("removing resource..." + index);
        for (Iterator i=parentTree.keySet().iterator(); i.hasNext(); ) {
            MutableTreeNode p = (MutableTreeNode)i.next();
            
            JTree rt = (JTree)parentTree.get(p);
            DefaultTreeModel tm = (DefaultTreeModel)rt.getModel();
            
            MutableTreeNode child = (MutableTreeNode)p.getChildAt(index);
            tm.removeNodeFromParent(child);
            //tm.removeNodeFromParent(child);
            
            //p.remove(index); // useless
            //tm.reload();
        }
        
    }
    
    public boolean addAll(int index, Collection c) {
        //fixme: trigger
        System.out.println("NYI: public boolean addAll(int index, Collection c)");
        return list.addAll(index, c);
    }
    
    public ListIterator listIterator(int index) {
        //fixme: NYI
        System.out.println("NYI: public ListIterator listIterator(int index)");
        return list.listIterator(index);
    }
    
    public boolean containsAll(Collection c) {
        return list.containsAll(c);
    }
    
    public int size() {
        return list.size();
    }
    
    public int indexOf(Object o) {
        return list.indexOf(o);
    }
    
    public boolean retainAll(Collection c) {
        //fixme: NYI
        System.out.println("NYI: public boolean retainAll(Collection c)");
        return list.retainAll(c);
    }
    
    public boolean contains(Object o) {
        return list.contains(o);
    }
    
    public boolean addAll(Collection c) {
        //fixme: NYI
        System.out.println("NYI: public boolean addAll(Collection c)");
        return list.addAll(c);
    }
    
}

