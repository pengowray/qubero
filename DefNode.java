import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;

/**
 * Node used by MoojTree containing display information for a file/definition.
 * also to be uesd by HexPanel?
 */

class DefNode extends DefaultMutableTreeNode {
    protected RawData rawdata;
    protected DefaultTreeModel def;
    protected DefaultMutableTreeNode selectionHeader = null; // the header/heading node for the current selection
    protected RawDataSelection currentSelection = null;
    protected DefaultMutableTreeNode currentSelectionNode = null;
    protected DefaultTreeModel parentTreeModel; // the tree model this is part of. XXX: multiple?
    protected HexPanel hexpanel; //XXX: allow multiple?

    /**
     * def may be null. in future rawdata may be null too (to indicate an empty file).
     */
    public DefNode(RawData rawdata, DefaultTreeModel def) {
	super(rawdata.toString());
	this.rawdata = rawdata;
	this.def = def;
    }

    public void setParentTreeModel(DefaultTreeModel parentTreeModel) {
	this.parentTreeModel = parentTreeModel;
	//XXX: rebuild self within this model?
    }

    public DefaultTreeModel getParentTreeModel() {
	return parentTreeModel; 
    }

    public void addHexPanel(HexPanel hexpanel) {
	//XXX: should remember more than one!
	this.hexpanel = hexpanel;
    }

    public void removeHexPanel(HexPanel hexpanel) {
	if (this.hexpanel == hexpanel) {
	    this.hexpanel = null;
	}
    }

    public void setSelection(RawDataSelection sel){
	//XXX: how does this work with selectionMade?
	currentSelection = sel;
    }

    // hexpanel selection:
    public void selectionMade(SelectionEvent e) {
	DefaultMutableTreeNode oldSelectionNode	= currentSelectionNode;

	if (selectionHeader == null) {
	    selectionHeader = new DefaultMutableTreeNode("Selection",true);
	    parentTreeModel.insertNodeInto(selectionHeader, this, 0); // XXX: should be at end really?
	}

	if (oldSelectionNode != null) {
	    //selectionHeader.remove(0); // XXX: remove from treemodel
	    parentTreeModel.removeNodeFromParent(oldSelectionNode);

	    //int x = selectionHeader.getIndex(currentSelectionNode);
	    //if (x != -1) {
	    //  selectionHeader.remove(x);
	    //}
	}

	currentSelection = e.getSelection();
	currentSelectionNode = new DefaultMutableTreeNode(currentSelection,false);
	parentTreeModel.insertNodeInto(currentSelectionNode, selectionHeader, 0);

	//XXX: put this back!
	//makeVisible(new TreePath(new Object[]{topnode,selectionHeader,currentSelectionNode}));
    }


    public RawDataSelection getSelection(){
	return currentSelection;
    }

    public RawData getRawData() {
	return rawdata;
    }

    public void setRawData(RawData rawdata) {
	//XXX: should trigger a few things
	this.rawdata = rawdata;
    }

    public String toString() {
	return "DefNode!" + rawdata.toString();
    }


}
