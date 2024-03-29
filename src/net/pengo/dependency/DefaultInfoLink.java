/**
 * DefaultLink.java
 *
 * @author  Peter Halasz
 */

package net.pengo.dependency;

import net.pengo.resource.*;

public class DefaultInfoLink implements InfoLink {
    public String verbA2B;
    public String verbB2A;
    public Resource nodeA;
    public Resource nodeB;
    
    public DefaultInfoLink(String verbA2B, String verbB2A, Resource nodeA, Resource nodeB) {
	this.verbA2B = verbA2B;
	this.verbB2A = verbB2A;
	this.nodeA = nodeA;
	this.nodeB = nodeB;
    }
    
    /**
     * Sets VerbA2B
     *
     * @param    VerbA2B             a  String
     */
    public void setVerbA2B(String verbA2B) {
	this.verbA2B = verbA2B;
    }
    
    /**
     * Returns VerbA2B
     *
     * @return    a  String
     */
    public String getVerbA2B() {
	return verbA2B;
    }
    
    /**
     * Sets VerbB2A
     *
     * @param    VerbB2A             a  String
     */
    public void setVerbB2A(String verbB2A) {
	this.verbB2A = verbB2A;
    }
    
    /**
     * Returns VerbB2A
     *
     * @return    a  String
     */
    public String getVerbB2A() {
	return verbB2A;
    }
    
    /**
     * Sets NodeA
     *
     * @param    NodeA               an InfoNode
     */
    public void setNodeA(Resource nodeA) {
	this.nodeA = nodeA;
    }
    
    /**
     * Returns NodeA
     *
     * @return    an InfoNode
     */
    public Resource getNodeA() {
	return nodeA;
    }
    
    /**
     * Sets NodeB
     *
     * @param    NodeB               an InfoNode
     */
    public void setNodeB(Resource nodeB) {
	this.nodeB = nodeB;
    }
    
    /**
     * Returns NodeB
     *
     * @return    an InfoNode
     */
    public Resource getNodeB() {
	return nodeB;
    }
    
}

