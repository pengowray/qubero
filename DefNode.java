import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;

/**
 * Node used by MoojTree containing display information for a file/definition.
 * also to be uesd by HexPanel?
 * //XXX: needs to be renamed (e.g. OpenFileNode or TopFileNode or something)
 */

class DefNode extends DefaultMutableTreeNode {
    protected RawData rawdata;
    protected DefaultTreeModel parentTreeModel; // the tree model this is part of. XXX: multiple?

    protected DefSection selectionHeader; // the header/heading node for the current selection
    protected DefSection definitionHeader; // the header node for the definition
    protected HexPanel hexpanel; //XXX: allow multiple?

    /**
     * def may be null. in future rawdata may be null too (to indicate an empty file).
     */
    public DefNode(RawData rawdata, DefaultTreeModel parentTreeModel) {
	super(rawdata.toString());
	this.rawdata = rawdata;
	this.parentTreeModel = parentTreeModel;
        
        selectionHeader = new DefSection("Selection");
        selectionHeader.setTreeModel(parentTreeModel, this, 0);
        definitionHeader = new DefSection("Definition");
        definitionHeader.setTreeModel(parentTreeModel, this, 1);
        
    }

    public void setParentTreeModel(DefaultTreeModel parentTreeModel) {
	this.parentTreeModel = parentTreeModel;
	//XXX: rebuild self within this model?
        //XXX: must alert DefSections (HeaderSections)
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

    // hexpanel selection:
    public void selectionMade(SelectionEvent e) {
	setSelection(e.getSelection());
    }

    public void setSelection(RawDataSelection sel){
        SectionedSelectionNode ssn = new SectionedSelectionNode(sel); //XXX: DefSection should be responsible for wrapping
        selectionHeader.setContents(ssn);
        
        /*
	DefaultMutableTreeNode oldSelectionNode	= currentSelectionNode;

	if (selectionHeader == null) {
	    selectionHeader = new DefaultMutableTreeNode("Selection",true);
	    parentTreeModel.insertNodeInto(selectionHeader, this, 0); // XXX: should be at end really?
	}

	if (oldSelectionNode != null) {
	    parentTreeModel.removeNodeFromParent(oldSelectionNode);
	}

	currentSelection = sel;
	currentSelectionNode = new DefaultMutableTreeNode(currentSelection,false);
	parentTreeModel.insertNodeInto(currentSelectionNode, selectionHeader, 0);

	//XXX: put this back!
	//makeVisible(new TreePath(new Object[]{topnode,selectionHeader,currentSelectionNode}));
        */
    }


    // MoojTree rename (edit) of node / converting selection to a definition:
    public void definitionMade(RawDataSelection sel) {
        //XXX: pending
        /*
	DefaultMutableTreeNode oldSelectionNode	= currentSelectionNode;

	if (definitionHeader == null) {
	    definitionHeader = new DefaultMutableTreeNode("Definition",true);
	    parentTreeModel.insertNodeInto(definitionHeader, this, 0);
	}

	DefaultMutableTreeNode definitionNode = new DefaultMutableTreeNode(sel,false);
	parentTreeModel.insertNodeInto(definitionNode, definitionHeader, 0);

	//makeVisible(new TreePath(new Object[]{topnode,selectionHeader,currentSelectionNode}));
        */
    }
    
    public void deleteDefinition(MutableTreeNode sel) {
	//XXX: take an actual selection, and find its TreeNode wrapper?
	parentTreeModel.removeNodeFromParent(sel);
    }

    public RawDataSelection getSelection(){
        //XXX pending
        return null;
	//return currentSelection;
    }

    public RawData getRawData() {
	return rawdata;
    }

    public void setRawData(RawData rawdata) {
	//XXX: should trigger a few things
	this.rawdata = rawdata;
    }

    public String toString() {
	return rawdata.toString();
	//return "DefNode " + rawdata.toString();
    }


}
