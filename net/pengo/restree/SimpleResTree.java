/**
 * SimpleResTree.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.restree;

// this tree doesn't have to worry about new resources being added, because they add themselves via ResourceList

import javax.swing.*;
import javax.swing.tree.*;

import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.event.MouseListener;
import javax.swing.event.TreeSelectionEvent;
import javax.swing.event.TreeSelectionListener;
import net.pengo.resource.Resource;
import net.pengo.app.OpenFile;

public class SimpleResTree extends JTree
{
    protected DefaultTreeModel treemodel;
    protected DefaultMutableTreeNode topnode;
	
	/**
	 * Method addOpenFile
	 *
	 * @param    of                  an OpenFile
	 *
	 */
	public void addOpenFile(OpenFile of)
	{
		of.getResourceList().addParent(this, topnode);
	}
	
	// the top node of the tree on the left, which the data goes under
    public static SimpleResTree create(OpenFile openFile)
	{
		DefaultMutableTreeNode topnode = new DefaultMutableTreeNode("mooj",true);
		DefaultTreeModel treemodel = new DefaultTreeModel(topnode);
		return new SimpleResTree(openFile, treemodel, topnode);
    }
	
    private SimpleResTree(OpenFile openFile, DefaultTreeModel treemodel, DefaultMutableTreeNode topnode)
	{
		super(treemodel);
		
		this.treemodel = treemodel;
		this.topnode = topnode;
		
		addOpenFile(openFile);
			
		getSelectionModel().setSelectionMode(TreeSelectionModel.SINGLE_TREE_SELECTION); //FIXME: for now
		
		/*
		 setRootVisible(false);
		setShowsRootHandles(false);
		setEditable(true);
		 */
		
		setRootVisible(true);
		setShowsRootHandles(true);
		setEditable(true);

		//treemodel.insertNodeInto(openFileNode, topnode, topnode.getChildCount());
		
		

		
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
				final int selRow = SimpleResTree.this.getRowForLocation(e.getX(), e.getY());
				final TreePath selPath = SimpleResTree.this.getPathForLocation(e.getX(), e.getY());
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
							popup.show(SimpleResTree.this, e.getX(), e.getY());
						}
						else
						{
							// default popup menu
							JMenu pop = new JMenu("default menu");
							Action ia = new InfoAction(selPath);
							pop.add(ia);
							popup = pop.getPopupMenu();
							popup.show(SimpleResTree.this, e.getX(), e.getY());
						}
					}
				}
			}
	    };
		addMouseListener(ml);
		
    }
	
	public DefaultMutableTreeNode getTopNode()
	{
		return topnode;
	}
	
}

