/*
 * ResourceSortedSet.java
 *
 * Created on July 11, 2003, 10:49 PM
 */

package net.pengo.restree;

import java.util.*;
import javax.swing.JTree;
import javax.swing.tree.DefaultMutableTreeNode;
import javax.swing.tree.MutableTreeNode;
import javax.swing.tree.TreePath;
import javax.swing.tree.DefaultTreeModel;

import net.pengo.app.OpenFile;
import net.pengo.resource.Resource;
import net.pengo.resource.ResourceFactory;
import net.pengo.resource.CollectionResource;

/**
 *
 * @author  Smiley
 */
public class ResourceSortedSet implements SortedSet {
    private SortedSet list;
    private ResourceFactory resFact;
    private String name;

    private Map parentTree = new HashMap(); // parent nodes for this resource list. MutableTreeNode->JTree. MutableTreeNode must not be modified by any other objects besides this.
    
    /** Creates a new instance of ResourceSortedSet */
    public ResourceSortedSet(SortedSet set, ResourceFactory resFact, String name) {
	this.list = set;
	setResourceFactory(resFact);
	this.name = name;
    }

    public ResourceSortedSet(SortedSet set, OpenFile openFile, String name) {
	this.list = set;
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
	if (resFact == null) {
	    resFact = ResourceFactory.getDefault();
	}
	
	this.resFact = resFact;
	refreshTree();
    }

    public String toString() {
	if (name == null) {
	    return super.toString();
	}
	
	return name;
    }
    
    public boolean add(Object o) {
        //fixme: doesn't place node in the right spot.
        
	boolean r = list.add(o);
	if (r) {
	    refreshTree();
	}
	
	return r;
    }
    
    public boolean addAll(Collection c) {
	boolean r = list.addAll(c);
        refreshTree();
	
	return r;
    }

    private void refreshTree() {
        // wipe parents
	for (Iterator i=parentTree.keySet().iterator(); i.hasNext(); ) {
	    MutableTreeNode p = (MutableTreeNode)i.next();
	    SimpleResTree jt = (SimpleResTree)parentTree.get(p);
            DefaultTreeModel tm = (DefaultTreeModel)jt.getModel();
            
            for (int ch = p.getChildCount()-1; ch>=0; ch--) {
                MutableTreeNode child = (MutableTreeNode)p.getChildAt(ch);
                tm.removeNodeFromParent(child);
            }
        }
        
        // rebuild parents
	for (Iterator i=parentTree.keySet().iterator(); i.hasNext(); ) {
	    MutableTreeNode p = (MutableTreeNode)i.next();
	    SimpleResTree jt = (SimpleResTree)parentTree.get(p);
            DefaultTreeModel tm = (DefaultTreeModel)jt.getModel();
            
            int count=0;
            for (Iterator it=list.iterator(); it.hasNext(); ) {
                Object o = it.next();
                insertNode(p, o, jt, count);
                count++;
            }
        }
        
    }
    
    private void insertNode(MutableTreeNode parent, Object childObject, SimpleResTree jt, int index) {
	Resource resourceObject = resFact.wrap(childObject);
	DefaultTreeModel tm = (DefaultTreeModel)jt.getModel();
	DefaultMutableTreeNode child = new DefaultMutableTreeNode(resourceObject);
	
	//p.insert(child, index); // not much good.
	tm.insertNodeInto(child, parent, index);
	jt.makeVisible(new TreePath(((DefaultTreeModel)jt.getModel()).getPathToRoot(child))); //fixme messy? nah
	subTreeCheck(jt, childObject, child);
    }
    
    private void subTreeCheck(SimpleResTree parent, Object childObject, MutableTreeNode childNode) {
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
    
    public void addParent(SimpleResTree rt, MutableTreeNode p) {
	// add existing items to new parent node
        int i=0;
        for (Iterator it=list.iterator(); it.hasNext(); ) {
            Object obj = it.next();
	    DefaultTreeModel tm = (DefaultTreeModel)rt.getModel();
	    //p.insert(child, i); // not much good.
	    insertNode(p, obj, rt, i);
            i++;
	}
	
	// add new parent node to list of parent nodes
	parentTree.put(p, rt);
    }

    public boolean removeAll(Collection c) {
	boolean r = list.removeAll(c);
        refreshTree();
        return r;
    }
    
    public void clear() {
        list.clear();
        refreshTree();
        return;
    }
    
    public Comparator comparator() {
        return list.comparator();
    }
    
    public boolean contains(Object o) {
        return list.contains(o);
    }
    
    public boolean containsAll(Collection c) {
        return list.containsAll(c);
    }
    
    public Object first() {
        return list.first();
    }
    
    public SortedSet headSet(Object toElement) {
        //fixme: subsorted set needs to be linked to this still.
        return list.headSet(toElement);
    }
    
    public boolean isEmpty() {
        return list.isEmpty();
    }
    
    public Iterator iterator() {
        //fixme: iterator needs to be linked to this still.
        return list.iterator();
    }
    
    public Object last() {
        return list.last();
    }
    
    public boolean remove(Object o) {
        boolean r = list.remove(o);
        refreshTree();
        return r;
    }
    
    public boolean retainAll(Collection c) {
        boolean r = list.retainAll(c);
        refreshTree();
        return r;
    }
    
    public int size() {
        return list.size();
    }
    
    public SortedSet subSet(Object fromElement, Object toElement) {
        //fixme: subsorted set needs to be linked to this still.
        return list.subSet(fromElement, toElement);
    }
    
    public SortedSet tailSet(Object fromElement) {
        //fixme: subset needs to be linked to this still.
        return list.tailSet(fromElement);
    }
    
    public Object[] toArray() {
        return list.toArray();
    }
    
    public Object[] toArray(Object[] a) {
        return list.toArray(a);
    }
    
}
