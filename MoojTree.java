import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;
import java.util.*;

class MoojTree extends JTree {
    protected HexPanel hexpanel = null;

    protected DefaultTreeModel treemodel;
    protected DefaultMutableTreeNode topnode; // the top node of the tree on the left, which the data goes under

    protected List defNodeList = new ArrayList(); // nodes for the display of data definitions + their selection

    // constructor
    public static MoojTree create(DefNode defnode) {
        DefaultMutableTreeNode topnode = new DefaultMutableTreeNode("mooj",true);

	DefaultTreeModel treemodel = new DefaultTreeModel(topnode);
	return new MoojTree(treemodel, defnode, topnode);
    }

    private MoojTree(DefaultTreeModel treemodel, DefNode defnode, DefaultMutableTreeNode topnode) {
	super(treemodel);

	this.treemodel = treemodel;
	this.topnode = topnode;

	getSelectionModel().setSelectionMode(TreeSelectionModel.SINGLE_TREE_SELECTION); //XXX: for now

	//setRootVisible(false); //XXX: re-enable.. Why won't it work?
	setShowsRootHandles(true);
	setEditable(true);

        addDefNode(defnode);


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

    public void addDefNode(DefNode defNode) {
	//XXX: error checking
	defNode.setParentTreeModel(treemodel);
	defNodeList.add(defNode);
	treemodel.insertNodeInto(defNode, topnode, 0); // XXX: put at end, not begining?
	//setRootVisible(false); // uncomment! grr.
    }

    public void removeDefNode(DefNode defNode) {
	if (defNode == null) 
	    return;

	defNodeList.remove(defNode);
	treemodel.removeNodeFromParent(defNode);
	if (defNodeList.isEmpty()) {
	    setRootVisible(true);
	}
    }

    public void setHexPanel(HexPanel hexpanel) {
	this.hexpanel = hexpanel;
    }

}
