import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;

class MoojTree extends JTree implements HexPanelListener {
    // protected JTree tree; // replaced by "this"
    protected DefaultTreeModel treemodel;
    protected DefaultMutableTreeNode def; // the definition of the hex data
    protected DefaultMutableTreeNode topnode; // the top node of the tree on the left, which the data goes under
    protected DefaultMutableTreeNode selectionHeader; // the "header" node for the [current] selection
    protected RawDataSelection currentSelection;
    protected DefaultMutableTreeNode currentSelectionNode = null;
    protected HexPanel hexpanel = null;

    // constructor
    public static MoojTree create(DefaultMutableTreeNode def) {
        DefaultMutableTreeNode topnode = new DefaultMutableTreeNode("mooj",true);
        topnode.add(def);

	DefaultTreeModel treemodel = new DefaultTreeModel(topnode);
	return new MoojTree(treemodel, def, topnode);
   }

    private MoojTree(DefaultTreeModel treemodel, DefaultMutableTreeNode def, DefaultMutableTreeNode topnode) {
	super(treemodel);

	this.def = def;
	this.treemodel = treemodel;
	this.topnode = topnode;

	setRootVisible(false);
	setShowsRootHandles(true);
	setEditable(true);

	getSelectionModel().setSelectionMode(TreeSelectionModel.SINGLE_TREE_SELECTION); //XXX: for now

	// click on tree:
	addTreeSelectionListener(new TreeSelectionListener() {
	    public void valueChanged(TreeSelectionEvent treeEvent) {
		DefaultMutableTreeNode node = (DefaultMutableTreeNode)getLastSelectedPathComponent();

		if (node == null) 
		    return;

		Object nodeInfo = node.getUserObject();

		if (nodeInfo instanceof RawDataSelection){
		    hexpanel.setSelection((RawDataSelection)nodeInfo);
		}
	    }
	});

    }

    public void setHexPanel(HexPanel hexpanel) {
	this.hexpanel = hexpanel;
    }

    // hexpanel selection:
    public void selectionMade(SelectionEvent e) {
	DefaultMutableTreeNode oldSelectionNode	= currentSelectionNode;

	if (selectionHeader == null) {
	    selectionHeader = new DefaultMutableTreeNode("Selection",true);
	    treemodel.insertNodeInto(selectionHeader, topnode, 1);
	}

	if (oldSelectionNode != null) {
	    //selectionHeader.remove(0); // XXX: remove from treemodel
	    treemodel.removeNodeFromParent(oldSelectionNode);

	    //int x = selectionHeader.getIndex(currentSelectionNode);
	    //if (x != -1) {
	    //  selectionHeader.remove(x);
	    //}
	}

	currentSelection = e.getSelection();
	currentSelectionNode = new DefaultMutableTreeNode(currentSelection,false);
	treemodel.insertNodeInto(currentSelectionNode, selectionHeader, 0);
	makeVisible(new TreePath(new Object[]{topnode,selectionHeader,currentSelectionNode}));
                ////topnode.add(oldSelectionNode);
                //jframe.getContentPane().remove(tree);
                //tree = makeTree(); //new JTree(topnode);
                //jframe.getContentPane().add(tree, BorderLayout.WEST);

	//XXX: restore:	
	//statusbar.setText(".." + currentSelection);
    }


}
