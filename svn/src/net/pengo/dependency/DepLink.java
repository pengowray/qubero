/**
 * DepLink.java
 *
 * @author  Peter Halasz
 */

package net.pengo.dependency;

import net.pengo.resource.*;

public interface DepLink
{
    public Resource getSource();
    public Resource getSink();
}


