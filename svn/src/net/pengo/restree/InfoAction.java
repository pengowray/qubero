/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * InfoAction.java
 *
 * @author  Peter Halasz
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
		super("Node info");
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

