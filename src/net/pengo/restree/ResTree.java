package net.pengo.restree;
import java.awt.event.ActionEvent;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.event.MouseListener;
import java.beans.PropertyChangeEvent;
import java.beans.PropertyChangeListener;
import java.beans.VetoableChangeListener;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.List;

import javax.swing.AbstractAction;
import javax.swing.Action;
import javax.swing.JMenu;
import javax.swing.JPopupMenu;
import javax.swing.JSeparator;
import javax.swing.JTree;
import javax.swing.event.TreeSelectionEvent;
import javax.swing.event.TreeSelectionListener;
import javax.swing.tree.DefaultMutableTreeNode;
import javax.swing.tree.DefaultTreeModel;
import javax.swing.tree.MutableTreeNode;
import javax.swing.tree.TreePath;
import javax.swing.tree.TreeSelectionModel;

import net.pengo.app.ClipboardEvent;
import net.pengo.app.FileEvent;
import net.pengo.app.OpenFile;
import net.pengo.app.OpenFileListener;
import net.pengo.app.SelectionEvent;
import net.pengo.data.DataEvent;
import net.pengo.resource.Resource;
import net.pengo.resource.ResourceEvent;
import net.pengo.resource.ResourceListener;

public class ResTree extends JTree implements ResourceListener, OpenFileListener
{
    protected DefaultTreeModel treemodel;
    protected DefaultMutableTreeNode topnode; // the top node of the tree on the left, which the data goes under
	
    protected List openFileList = new ArrayList(); // open files to display (+ their resources)
    protected List topNodeList = new ArrayList(); // TreeNode wrapped versions of above. keep in sync.
    
    protected List resList = new ArrayList(); // resources
    protected List resNodeList = new ArrayList(); // resources as nodes
	
    protected final ResTree thisTree = this; // needed for mouse adapter
	
	
    // constructor
    public static ResTree create()
	{
		return create(null);
    }
	
    public static ResTree create(OpenFile openFile)
	{
		DefaultMutableTreeNode topnode = new DefaultMutableTreeNode("mooj",true);
		DefaultTreeModel treemodel = new DefaultTreeModel(topnode);
		return new ResTree(openFile, treemodel, topnode);
    }
	
    private ResTree(OpenFile openFile, DefaultTreeModel treemodel, DefaultMutableTreeNode topnode)
	{
		super(treemodel);
		
		this.treemodel = treemodel;
		this.topnode = topnode;
		
		getSelectionModel().setSelectionMode(TreeSelectionModel.SINGLE_TREE_SELECTION); //FIXME: for now
		
		setRootVisible(false);
		setShowsRootHandles(false);
		setEditable(true);
		
		//testRenameTest(); // debugging
		
		if (openFile != null)
			addOpenFile(openFile);
		
		// click on tree:
		addTreeSelectionListener(new TreeSelectionListener()
								 {
					public void valueChanged(TreeSelectionEvent treeEvent)
					{
						DefaultMutableTreeNode node = (DefaultMutableTreeNode)getLastSelectedPathComponent();
						if (node == null)
						{
							return;
						}
						
						Object innerSelected = node.getUserObject();
						if (innerSelected instanceof Resource)
						{
							Resource res = (Resource)innerSelected;
							res.clickAction();
						}
						else
						{
							// nothing
						}
					}
				});
		
		// popup menu
		// FIXME: take menu from SectionedNode.getPopupMenu()
		MouseListener ml = new MouseAdapter()
		{
			public void mousePressed(MouseEvent e)
			{
				final int selRow = thisTree.getRowForLocation(e.getX(), e.getY());
				final TreePath selPath = thisTree.getPathForLocation(e.getX(), e.getY());
				if (selPath == null)
					return;
				
				final Object[] o = selPath.getPath();
				final DefaultMutableTreeNode selected = (DefaultMutableTreeNode)o[o.length-1];
				final Object innerSelected = selected.getUserObject();
				
				if(selRow != -1)
				{
					if (e.getClickCount() == 2 && e.isMetaDown() == false)
					{
						if (innerSelected instanceof Resource)
						{
							Resource res = (Resource)innerSelected;
							res.doubleClickAction();
						}
					}
					else if (e.isPopupTrigger() || e.isMetaDown() ) {//FIXME: popup trigger is never true? why?
						setLeadSelectionPath(new TreePath(o)); // highlight (ant trail) selection. FIXME: doesn't always work
						setAnchorSelectionPath(new TreePath(o)); // highlight selection. FIXME: doesn't always work
						JPopupMenu popup;
						/* // put this back if Resource becomes a type of DefaultMutableTreeNode (unlikely)
						 if (selected instanceof Resource) {
						 popup = ((Resource)selected).getJMenu().getPopupMenu();
						 } else */
						if (innerSelected instanceof Resource)
						{
							popup = ((Resource)innerSelected).getJMenu().getPopupMenu();
							popup.add(new InfoAction(selPath));
							popup.show(thisTree, e.getX(), e.getY());
						}
						else
						{
							// default popup menu
							JMenu pop = new JMenu("default menu");
							Action ia = new InfoAction(selPath);
							pop.add(new JSeparator());
							pop.add(ia);
							popup = pop.getPopupMenu();
							popup.show(thisTree, e.getX(), e.getY());
						}
					}
				}
			}
	    };
		addMouseListener(ml);
		
    }
    
    // find out what's happening when someone renames a tree element!
    public void testRenameTest()
	{
		if (getCellEditor() != null)
		{
			System.out.println( "ed: " + getCellEditor() + " .. " + getCellEditor().getCellEditorValue());
		}
		
		this.addPropertyChangeListener( new PropertyChangeListener()
									   {
					public void propertyChange(PropertyChangeEvent evt)
					{
						System.out.println(evt.getPropertyName() + " -- " + evt.getOldValue() + " -> " + evt.getNewValue());
					}
				});
		
		this.addVetoableChangeListener( new VetoableChangeListener()
									   {
					public void vetoableChange(PropertyChangeEvent evt)
					{
						System.out.println("Vetoable! " + evt.getPropertyName() + " -- " + evt.getOldValue() + " -> " + evt.getNewValue());
					}
				});
    }
	
    // add a viewed file, effectively:
    public void addOpenFile(OpenFile openFile)
	{
		//FIXME: error checking?
		
		openFile.addResourceListener(this);
		openFile.addOpenFileListener(this);
		openFileList.add(openFile);
		
		// wrap it!
		DefaultMutableTreeNode openFileNode = new DefaultMutableTreeNode(openFile,true);
		topNodeList.add(openFileNode);
		
		treemodel.insertNodeInto(openFileNode, topnode, topnode.getChildCount());
		makeVisible(new TreePath(treemodel.getPathToRoot(openFileNode)));
		
		setRootVisible(false);
    }
    
    public void fileClosed(FileEvent e)
	{
		removeOpenFile(e.getOpenFile());
    }
	
	
    public void removeOpenFile(OpenFile openFile)
	{
		if (openFile == null)
			return;
		
		openFile.removeResourceListener(this);
		openFile.removeOpenFileListener(this);
		int index = openFileList.indexOf(openFile);
		openFileList.remove(index);
		DefaultMutableTreeNode treenode = (DefaultMutableTreeNode)topNodeList.remove(index);
		treemodel.removeNodeFromParent(treenode);
		
		if (openFileList.isEmpty())
		{
			setRootVisible(true);
		}
    }
    
    public void resourceAdded(ResourceEvent e)
	{
		//FIXME: shouldn't really ignore category, but will for now
		Resource res = e.getResource();
		String cat = e.getCategory();
		OpenFile of = e.getOpenFile();
		
		DefaultMutableTreeNode resNode = new DefaultMutableTreeNode(res); //FIXME: need res.canHaveChildren()?
		
		resList.add(res);
		resNodeList.add(resNode);
		
		int index = openFileList.indexOf(of);
		DefaultMutableTreeNode openFileNode = (DefaultMutableTreeNode)topNodeList.get(index);
		DefaultMutableTreeNode catNode = findOrAdd(openFileNode, cat);
		
		treemodel.insertNodeInto(resNode,catNode,catNode.getChildCount()); // FIXME: position should be explicit
		
		makeVisible(new TreePath(treemodel.getPathToRoot(resNode)));
    }
    
    protected DefaultMutableTreeNode findChild(DefaultMutableTreeNode parent, String child)
	{
		for (Enumeration enum = parent.children(); enum.hasMoreElements(); )
		{
			DefaultMutableTreeNode node = (DefaultMutableTreeNode)enum.nextElement();
			if (child.equals(node.getUserObject()))
			{
				return node;
			}
		}
		return null;
		
    }
    protected DefaultMutableTreeNode findOrAdd(DefaultMutableTreeNode parent, String child)
	{
		DefaultMutableTreeNode find = findChild(parent,child);
		if (find != null)
			return find;
		
		DefaultMutableTreeNode newChild = new DefaultMutableTreeNode(child);
		treemodel.insertNodeInto(newChild,parent,parent.getChildCount());
		return newChild;
    }
    
    public void resourceMoved(ResourceEvent e)
	{
    }
    
    public void resourceRemoved(ResourceEvent e)
	{
		Resource res = e.getResource();
		String cat = e.getCategory();
		OpenFile of = e.getOpenFile();
		int index = resList.indexOf(res);
		if (index < 0)
			return;
		MutableTreeNode resNode = (MutableTreeNode)resNodeList.get( index );
		
		treemodel.removeNodeFromParent(resNode); //works but trying the following instead:
		//resNode.removeFromParent();
		
		resList.remove(index);
		resNodeList.remove(index);
		
		//FIXME: could alternatively search up the path of the resource for the catNode
		int ofIndex = openFileList.indexOf(of);
		DefaultMutableTreeNode openFileNode = (DefaultMutableTreeNode)topNodeList.get(ofIndex);
		DefaultMutableTreeNode catNode = findChild(openFileNode, cat);
		if (catNode.getChildCount() == 0)
		{
			treemodel.removeNodeFromParent(catNode);
		}
    }
    
    public void dataEdited(DataEvent e)
	{
    }
    
    public void dataLengthChanged(DataEvent e)
	{
    }
    
    public void fileSaved(FileEvent e)
	{
    }
    
    public void selectionCopied(ClipboardEvent e)
	{
    }
    
    public void selectionMade(SelectionEvent e)
	{
    }
    
    public void selectionCleared(SelectionEvent e)
	{
    }
    
    public void resourcePropertyChanged(ResourceEvent e)
	{
    }
    
    public void resourceTypeChanged(ResourceEvent e)
	{
    }
    
}


class DeleteDefinitionAction extends AbstractAction
{
    protected TreePath selPath;
	
    public DeleteDefinitionAction(TreePath selPath)
	{
		super("Delete definition");
		this.selPath = selPath;
    }
	
    public void actionPerformed(ActionEvent e)
	{
		/*
		 //FIXME: this code is almost duplicated from above
		 Object[] path = selPath.getPath();
		 Object object = selPath.getLastPathComponent();
		 Object userObject = null;
		
		 if (object instanceof DefaultMutableTreeNode) {
		 userObject = ((DefaultMutableTreeNode)object).getUserObject();
		
		 if (userObject instanceof RawDataSelection) {
		 //FIXME: different menus for selection and definition
		 //if (path.length >= 2 && path[]);
		 
		 RawDataSelection raw = (RawDataSelection)userObject;
		 DefNode defnode = raw.getDefNode();
		 
		 defnode.deleteDefinition((DefaultMutableTreeNode)object);
		 //defnode.clearSelection();
		 }
		 }
		 */
		
    }
}



// do something with a tree node.
/*
 abstract class NodeVisitor {
 public NodeVisitor (TreePath selPath) {
 this.selPath = selPath;
 }

 public void actionPerformed(TreePath selPath) {
 Object[] path = selPath.getPath();
 Object object = selPath.getLastPathComponent();
 Object userObject = null;

 if (object instanceof DefaultMutableTreeNode) {
 userObject = ((DefaultMutableTreeNode)object).getUserObject();

 if (userObject instanceof RawDataSelection) {
 RawDataSelection raw = (RawDataSelection)userObject;
 DefNode defnode = raw.getDefNode();
 
 
 defnode.deleteDefinition((DefaultMutableTreeNode)object);
 actionPerformedOnDefinition(e, raw); //FIXME: blah
 }
 }
 }

 public void visitSelection(RawDataSelection raw){}

 public void visitDefinition(RawDataSelection raw){}

 public void visitEditedArea(ActionEvent e, RawDataSelection raw){}

 }
 */
