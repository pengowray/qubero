import java.util.*;
import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;

/**
 * Contains information about a section (Definition / Selection / Edited Area / Custom Types) 
 * and the commands for adding sectioned nodes, and creating a tree view, and reorganizing sectioned nodes.
 */

// was: HeaderDef

// MAKE IT WORK FIRST. 

// SECOND MAKE IT WORK WELL.


// --- This Class is Obsolete. ---


class SectionHeader {
    protected String name;
    protected DefaultMutableTreeNode sectionHeader; // the section header node

    protected DefaultTreeModel treeModel; //XXX: allow multiple?
    protected DefaultTreeModel dataTreeModel; // to use if section isn't connected to anything
    protected DefaultMutableTreeNode parentNode; // where to add this heading.
    protected int insertionPosition; // where on parent to insert. -1 for end.

    protected List allNodes = new ArrayList(); // SectionedNode

    //protected NodeWrapperFactory wrapper;
    
    //XXX: needed: option to auto-hide section when it is empty.

    public SectionHeader(String name) {
	this.name = name;
	sectionHeader = new DefaultMutableTreeNode(name,true);
    }

    public void setTreeModel(DefaultTreeModel treeModel, DefaultMutableTreeNode parentNode, int insertionPosition) {
	removeSection(); // removed from the old. XXX: allow multiple tree models?
        this.dataTreeModel = new DefaultTreeModel(new DefaultMutableTreeNode("Top"));
        this.treeModel = treeModel;
        this.parentNode = parentNode;
        this.insertionPosition = insertionPosition;
        System.out.println(treeModel + " -- " + sectionHeader + " -- " + parentNode + " -- " + insertionPosition);
	treeModel.insertNodeInto(sectionHeader, parentNode, insertionPosition); //XXX: negative insertionPosition->from end
	/*
	Iterator i = topNodes.iterator();
	for (; i.isNext(); node = (TreeNode)i.next()) {
	    
	}
	*/
    }

    // parent == null for top nodes
    public void insertNode(SectionedNode node, SectionedNode parent, int insertionPosition){
        if (parent == null) {
        	treeModel.insertNodeInto(node, sectionHeader, insertionPosition);
                return;
        }
        
	treeModel.insertNodeInto(node, parent, insertionPosition);	
    }

    /* deletes all sectioned nodes and replaces with new one */
    public void setContents(SectionedNode node){
        //XXX: delete old nodes
        appendNode(node);
    }
    
    public void appendNode(SectionedNode node) {
	treeModel.insertNodeInto(node, sectionHeader, 0); //XXX: position should be -1
    }

    public byte[] saveStructure(){
	// tricky?
	return new byte[0];
    }

    //public moveNode(node, SectionedNode newParent_or_sameParent, newPosition_or_relativePosition) {
    //	DnD would use SectionedNodes.
    //    SectionNodes would too.
    //}

    public void removeSection() { // remove this section from the treeModel.
	if (treeModel == null) 
	    return;

	treeModel.removeNodeFromParent(sectionHeader);
	treeModel = null;
	parentNode = null;
        insertionPosition = 0;
    }

    public String toString() {
	return name;
    }

    
}
