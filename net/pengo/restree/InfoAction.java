/**
 * InfoAction.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.restree;

import java.awt.event.ActionEvent;
import javax.swing.AbstractAction;
import javax.swing.tree.DefaultMutableTreeNode;
import javax.swing.tree.TreePath;

public class InfoAction extends AbstractAction
{
    protected TreePath selPath;
	
    public InfoAction(TreePath selPath)
	{
		super("Show info (Debug)");
		this.selPath = selPath;
    }
	
    public void actionPerformed(ActionEvent e)
	{
		Object object = selPath.getLastPathComponent();
		Object userObject = null;
		System.out.println("this is: " + object + " -- type: " + object.getClass()) ;
		
		if (object instanceof DefaultMutableTreeNode)
		{
			DefaultMutableTreeNode treenode = (DefaultMutableTreeNode)object;
			userObject = treenode.getUserObject();
			System.out.println("  children: " + treenode.getChildCount());
			System.out.println("  user object: " + userObject + " -- type: " + userObject.getClass());
			
		}
    }
}

